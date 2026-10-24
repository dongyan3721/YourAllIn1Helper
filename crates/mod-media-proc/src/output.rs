use app_core::{CoreError, CoreResult};
use parking_lot::Mutex;
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    sync::OnceLock,
};

static RESERVED_OUTPUTS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

fn reservations() -> &'static Mutex<HashSet<PathBuf>> {
    RESERVED_OUTPUTS.get_or_init(|| Mutex::new(HashSet::new()))
}

pub fn prepare_output(
    input: &Path,
    output_dir: Option<&Path>,
    extension: &str,
    job_id: &str,
) -> CoreResult<(PathBuf, PathBuf)> {
    let directory = output_dir
        .map(Path::to_path_buf)
        .or_else(|| input.parent().map(Path::to_path_buf))
        .ok_or_else(|| CoreError::Config("cannot determine output directory".into()))?;
    fs::create_dir_all(&directory)?;
    if !directory.is_dir() {
        return Err(CoreError::Config(format!(
            "output directory is not a directory: {}",
            directory.display()
        )));
    }
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("media");
    let mut reserved = reservations().lock();
    for index in 0..10_000_u32 {
        let suffix = if index == 0 {
            String::new()
        } else {
            format!("_{index}")
        };
        let output = directory.join(format!("{stem}_media{suffix}.{extension}"));
        let partial = directory.join(format!(
            ".{stem}_media{suffix}.{job_id}.partial.{extension}"
        ));
        if output.exists() || partial.exists() || reserved.contains(&output) {
            continue;
        }
        // Atomically reserve this unique partial path. FFmpeg receives -n as a second guard.
        let reservation = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)?;
        drop(reservation);
        fs::remove_file(&partial)?;
        // Keep the final name reserved for the whole job, including queued jobs.
        reserved.insert(output.clone());
        return Ok((output, partial));
    }
    Err(CoreError::Config(format!(
        "could not allocate a unique output name in {}",
        directory.display()
    )))
}

pub fn finalize_output(partial: &Path, output: &Path) -> CoreResult<()> {
    let result = (|| {
        // hard_link fails if output appeared concurrently, so an existing user file is never replaced.
        fs::hard_link(partial, output).map_err(|error| {
            CoreError::Io(std::io::Error::new(
                error.kind(),
                format!("refusing to overwrite {}: {error}", output.display()),
            ))
        })?;
        fs::remove_file(partial)?;
        Ok(())
    })();
    release_output(output);
    result
}

pub fn release_output(output: &Path) {
    reservations().lock().remove(output);
}

pub fn remove_partial(partial: &Path) {
    if partial.exists() {
        let _ = fs::remove_file(partial);
    }
}

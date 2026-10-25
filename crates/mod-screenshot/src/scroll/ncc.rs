//! 归一化互相关（NCC）与行特征匹配算法。
//!
//! 用于滚动长截图场景中精确计算相邻帧之间的纵向物理位移量 $\Delta y$，
//! 并识别固定页眉（Navigation Bar）与固定页脚（Status Bar / Dock）。

/// 匹配结果，包含纵向位移与置信度。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplacementMatch {
    /// 纵向位移量（像素）。$\Delta y > 0$ 表示向下滚动（新内容在下方出现）。
    pub dy: usize,
    /// 归一化互相关系数（0.0 ~ 1.0）。
    pub confidence: f64,
}

/// 匹配参数配置。
#[derive(Debug, Clone)]
pub struct NccConfig {
    /// 模板高度（像素），默认 60。
    pub template_height: usize,
    /// 最低置信度阈值（低于此值判定为低置信度，不采用），默认 0.82。
    pub min_confidence: f64,
    /// 左右边界剔除比例（避免网页或窗口右侧滚动条干扰匹配），默认 0.05（两边各裁 5%）。
    pub margin_ratio: f64,
    /// 固定头尾探测的最大高度占比，默认 0.25。
    pub max_fixed_ratio: f64,
    /// 像素平均绝对差容差（判断行是否完全静止），默认 1.5。
    pub static_row_tolerance: f64,
}

impl Default for NccConfig {
    fn default() -> Self {
        Self {
            template_height: 60,
            min_confidence: 0.82,
            margin_ratio: 0.05,
            max_fixed_ratio: 0.25,
            static_row_tolerance: 1.5,
        }
    }
}

/// 探测两帧之间的固定页眉高度（顶部静止区域）。
///
/// 从第 0 行向下逐行比对，若行内像素差均值低于容差，则视为固定页眉。
pub fn detect_fixed_header(
    prev: &[u8],
    curr: &[u8],
    width: usize,
    height: usize,
    config: &NccConfig,
) -> usize {
    let max_scan = ((height as f64) * config.max_fixed_ratio) as usize;
    let mut fixed_rows = 0;

    for y in 0..max_scan.min(height) {
        if is_row_identical(prev, curr, y, width, config.static_row_tolerance) {
            fixed_rows += 1;
        } else {
            break;
        }
    }
    fixed_rows
}

/// 探测两帧之间的固定页脚高度（底部静止区域）。
///
/// 从最后一行向上逐行比对，若行内像素差均值低于容差，则视为固定页脚。
pub fn detect_fixed_footer(
    prev: &[u8],
    curr: &[u8],
    width: usize,
    height: usize,
    config: &NccConfig,
) -> usize {
    let max_scan = ((height as f64) * config.max_fixed_ratio) as usize;
    let mut fixed_rows = 0;

    for i in 0..max_scan.min(height) {
        let y = height - 1 - i;
        if is_row_identical(prev, curr, y, width, config.static_row_tolerance) {
            fixed_rows += 1;
        } else {
            break;
        }
    }
    fixed_rows
}

/// 判断某一行在两帧中是否完全静止。
fn is_row_identical(prev: &[u8], curr: &[u8], y: usize, width: usize, tolerance: f64) -> bool {
    let row_start = y * width * 4;
    let row_end = row_start + width * 4;
    if row_end > prev.len() || row_end > curr.len() {
        return false;
    }

    let p_slice = &prev[row_start..row_end];
    let c_slice = &curr[row_start..row_end];

    let mut diff_sum: u64 = 0;
    for x in 0..width {
        let idx = x * 4;
        let dr = (p_slice[idx] as i32 - c_slice[idx] as i32).abs() as u64;
        let dg = (p_slice[idx + 1] as i32 - c_slice[idx + 1] as i32).abs() as u64;
        let db = (p_slice[idx + 2] as i32 - c_slice[idx + 2] as i32).abs() as u64;
        diff_sum += (dr + dg + db) / 3;
    }

    let mae = (diff_sum as f64) / (width.max(1) as f64);
    mae <= tolerance
}

/// 计算单行的平均亮度（0.0 ~ 255.0）。
fn row_mean_luminance(frame: &[u8], y: usize, width: usize, x_start: usize, x_end: usize) -> f64 {
    let row_start = y * width * 4;
    let mut sum = 0.0;
    let count = x_end.saturating_sub(x_start).max(1);

    for x in x_start..x_end {
        let idx = row_start + x * 4;
        if idx + 2 < frame.len() {
            let r = frame[idx] as f64;
            let g = frame[idx + 1] as f64;
            let b = frame[idx + 2] as f64;
            sum += 0.299 * r + 0.587 * g + 0.114 * b;
        }
    }
    sum / (count as f64)
}

/// 基于行特征初筛 + NCC 精准求解两帧之间的纵向滚动位移量 $\Delta y$。
///
/// # 参数
/// - `prev`: 前一帧 RGBA 像素
/// - `curr`: 当前帧 RGBA 像素
/// - `width`: 帧宽
/// - `height`: 帧高
/// - `header`: 已知或检测出的固定页眉高度
/// - `footer`: 已知或检测出的固定页脚高度
/// - `config`: NCC 配置参数
pub fn calculate_displacement(
    prev: &[u8],
    curr: &[u8],
    width: usize,
    height: usize,
    header: usize,
    footer: usize,
    config: &NccConfig,
) -> Option<DisplacementMatch> {
    let active_height = height.saturating_sub(header + footer);
    if active_height < config.template_height * 2 || width < 10 {
        return None;
    }

    // 左右边距裁切，避免垂直滚动条对图像相关性造成干扰
    let x_margin = ((width as f64) * config.margin_ratio) as usize;
    let x_start = x_margin;
    let x_end = width.saturating_sub(x_margin);
    if x_end <= x_start {
        return None;
    }

    let tmpl_h = config.template_height.min(active_height / 3).max(10);

    // 模板取自前一帧活动区域靠底部位置（排除固定页脚）
    let tmpl_y_in_prev = height - footer - tmpl_h;

    // 提取模板区域像素灰度图并计算统计量
    let tmpl_len = (x_end - x_start) * tmpl_h;
    let mut tmpl_vals = Vec::with_capacity(tmpl_len);
    let mut tmpl_sum = 0.0;
    let mut tmpl_sq_sum = 0.0;

    for y in tmpl_y_in_prev..(tmpl_y_in_prev + tmpl_h) {
        let row_start = y * width * 4;
        for x in x_start..x_end {
            let idx = row_start + x * 4;
            let gray = 0.299 * (prev[idx] as f64)
                + 0.587 * (prev[idx + 1] as f64)
                + 0.114 * (prev[idx + 2] as f64);
            tmpl_vals.push(gray);
            tmpl_sum += gray;
            tmpl_sq_sum += gray * gray;
        }
    }

    let n = tmpl_len as f64;
    let tmpl_mean = tmpl_sum / n;
    let tmpl_variance = (tmpl_sq_sum / n) - (tmpl_mean * tmpl_mean);

    // 如果模板区域方差极低（如纯白/纯黑/单色背景），无法形成唯一特征指纹
    if tmpl_variance < 4.0 {
        // 退化处理：尝试检查是否为完全重复帧（零位移）
        if is_region_identical(prev, curr, width, header, height - footer) {
            return Some(DisplacementMatch {
                dy: 0,
                confidence: 1.0,
            });
        }
        return None;
    }

    let tmpl_std = tmpl_variance.sqrt();

    // 预计算前一帧与当前帧在搜索范围内的行平均亮度，用于粗筛
    let mut prev_row_means = Vec::with_capacity(tmpl_h);
    for y in tmpl_y_in_prev..(tmpl_y_in_prev + tmpl_h) {
        prev_row_means.push(row_mean_luminance(prev, y, width, x_start, x_end));
    }

    let mut curr_row_means = Vec::with_capacity(height);
    for y in 0..height {
        curr_row_means.push(row_mean_luminance(curr, y, width, x_start, x_end));
    }

    // 搜索位移范围：dy 从 0 到 (height - header - footer - tmpl_h)
    let max_dy = height.saturating_sub(header + footer + tmpl_h);

    // 1. 行亮度粗筛：找出绝对差之和最小的前若干个位移候选
    let mut candidates: Vec<(usize, f64)> = Vec::with_capacity(max_dy + 1);

    for dy in 0..=max_dy {
        let cand_y = tmpl_y_in_prev.saturating_sub(dy);
        if cand_y < header {
            continue;
        }

        let mut row_diff = 0.0;
        for i in 0..tmpl_h {
            row_diff += (prev_row_means[i] - curr_row_means[cand_y + i]).abs();
        }
        candidates.push((dy, row_diff));
    }

    // 按行特征差异升序排列
    candidates.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    // 2. 精确 NCC 计算：对粗筛排名前 15 的候选进行归一化互相关匹配
    let check_count = candidates.len().min(15);
    let mut best_match: Option<DisplacementMatch> = None;

    for (dy, _) in candidates.into_iter().take(check_count) {
        let cand_y = tmpl_y_in_prev.saturating_sub(dy);
        let mut curr_sum = 0.0;
        let mut curr_sq_sum = 0.0;
        let mut cross_sum = 0.0;

        let mut val_idx = 0;
        for y in cand_y..(cand_y + tmpl_h) {
            let row_start = y * width * 4;
            for x in x_start..x_end {
                let idx = row_start + x * 4;
                let c_gray = 0.299 * (curr[idx] as f64)
                    + 0.587 * (curr[idx + 1] as f64)
                    + 0.114 * (curr[idx + 2] as f64);
                let t_gray = tmpl_vals[val_idx];
                val_idx += 1;

                curr_sum += c_gray;
                curr_sq_sum += c_gray * c_gray;
                cross_sum += t_gray * c_gray;
            }
        }

        let curr_mean = curr_sum / n;
        let curr_variance = (curr_sq_sum / n) - (curr_mean * curr_mean);
        if curr_variance < 1.0 {
            continue;
        }

        let curr_std = curr_variance.sqrt();
        let covariance = (cross_sum / n) - (tmpl_mean * curr_mean);
        let ncc = (covariance / (tmpl_std * curr_std)).clamp(-1.0, 1.0);

        if ncc >= config.min_confidence {
            match &best_match {
                Some(prev_best) if ncc > prev_best.confidence => {
                    best_match = Some(DisplacementMatch {
                        dy,
                        confidence: ncc,
                    });
                }
                None => {
                    best_match = Some(DisplacementMatch {
                        dy,
                        confidence: ncc,
                    });
                }
                _ => {}
            }
        }
    }

    best_match
}

/// 检查给定区域是否两帧完全相同。
fn is_region_identical(
    prev: &[u8],
    curr: &[u8],
    width: usize,
    y_start: usize,
    y_end: usize,
) -> bool {
    let start_byte = y_start * width * 4;
    let end_byte = y_end * width * 4;
    if end_byte > prev.len() || end_byte > curr.len() {
        return false;
    }
    prev[start_byte..end_byte] == curr[start_byte..end_byte]
}

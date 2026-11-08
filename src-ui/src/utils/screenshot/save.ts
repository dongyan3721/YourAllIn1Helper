/** 统一保存流程：取消路径选择时不写文件，默认保存绕过对话框。 */
export async function runSaveFlow<T>(
  useDefault: boolean,
  choosePath: () => Promise<string | null>,
  write: (path?: string) => Promise<T>,
): Promise<T | null> {
  const path = useDefault ? undefined : await choosePath();
  if (path === null) return null;
  return write(path);
}

/** 请求头必须为 ASCII；JSON 的 Unicode 转义保留中文路径、emoji 和反斜杠原值。 */
export function encodeExportMetadata(value: object): string {
  return JSON.stringify(value).replace(/[\u007f-\uffff]/g,
    char => `\\u${char.charCodeAt(0).toString(16).padStart(4, '0')}`);
}

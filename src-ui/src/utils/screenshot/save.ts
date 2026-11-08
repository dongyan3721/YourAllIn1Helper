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

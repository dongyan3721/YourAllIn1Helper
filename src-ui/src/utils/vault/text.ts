import { currentLocale, type SupportedLocale } from '../i18n';

const messages: Record<SupportedLocale, Record<string, string>> = {
  'zh-CN': {
    title: '加密密码本', m1: 'M1 主密码模式 · 安全审查前仅用于虚构数据',
    initialize: '初始化密码本', unlock: '使用主密码解锁', lock: '立即锁定',
    master: '主密码', confirm: '确认主密码', oldMaster: '当前主密码', newMaster: '新主密码',
    changeMaster: '修改主密码', changeMasterHint: '至少 12 个字符；成功后立即锁定',
    search: '搜索标题、用户名、网址、分类或备注', searchButton: '搜索', newEntry: '新建条目',
    noEntries: '没有匹配的条目', edit: '编辑', reveal: '揭示密码', hide: '隐藏密码',
    save: '保存条目', cancel: '取消', delete: '删除', deleteConfirm: '确定删除此虚构条目？',
    titleField: '标题', url: '网址', category: '分类', username: '用户名', password: '密码',
    totp: 'TOTP 密钥', notes: '备注', archived: '归档', generate: '生成密码',
    generated: '生成结果（15 秒后清除）', useGenerated: '填入编辑表单',
    copyDisabled: '秘密复制禁用：共享剪贴板租约尚未完成安全审查',
    biometricDisabled: '生物识别禁用：真实设备钥匙保护属于 M2',
    lockedNotice: '密码本已锁定，本地敏感状态已清理', busy: '正在安全处理…',
  },
  'en-US': {
    title: 'Encrypted Vault', m1: 'M1 master-password mode · fictional data only pending security review',
    initialize: 'Initialize vault', unlock: 'Unlock with master password', lock: 'Lock now',
    master: 'Master password', confirm: 'Confirm master password', oldMaster: 'Current master password', newMaster: 'New master password',
    changeMaster: 'Change master password', changeMasterHint: 'At least 12 characters; locks immediately after success',
    search: 'Search title, username, URL, category, or notes', searchButton: 'Search', newEntry: 'New entry',
    noEntries: 'No matching entries', edit: 'Edit', reveal: 'Reveal password', hide: 'Hide password',
    save: 'Save entry', cancel: 'Cancel', delete: 'Delete', deleteConfirm: 'Delete this fictional entry?',
    titleField: 'Title', url: 'URL', category: 'Category', username: 'Username', password: 'Password',
    totp: 'TOTP secret', notes: 'Notes', archived: 'Archived', generate: 'Generate password',
    generated: 'Generated result (clears after 15 seconds)', useGenerated: 'Use in edit form',
    copyDisabled: 'Secret copy disabled: shared clipboard lease review is incomplete',
    biometricDisabled: 'Biometrics disabled: real device key protection belongs to M2',
    lockedNotice: 'Vault locked; local sensitive state cleared', busy: 'Processing securely…',
  },
  'ja-JP': {
    title: '暗号化パスワード保管庫', m1: 'M1 マスターパスワード · 審査完了までは仮想データのみ',
    initialize: 'vault を初期化', unlock: 'マスターパスワードでロック解除', lock: '今すぐロック',
    master: 'マスターパスワード', confirm: 'マスターパスワード（確認）', oldMaster: '現在のマスターパスワード', newMaster: '新しいマスターパスワード',
    changeMaster: 'マスターパスワードを変更', changeMasterHint: '12 文字以上。成功後は直ちにロックします',
    search: 'タイトル・ユーザー名・URL・分類・メモを検索', searchButton: '検索', newEntry: '新規エントリ',
    noEntries: '一致するエントリがありません', edit: '編集', reveal: 'パスワードを表示', hide: 'パスワードを隠す',
    save: '保存', cancel: 'キャンセル', delete: '削除', deleteConfirm: 'この仮想エントリを削除しますか？',
    titleField: 'タイトル', url: 'URL', category: '分類', username: 'ユーザー名', password: 'パスワード',
    totp: 'TOTP シークレット', notes: 'メモ', archived: 'アーカイブ', generate: 'パスワードを生成',
    generated: '生成結果（15 秒後に消去）', useGenerated: '編集フォームに入力',
    copyDisabled: 'コピー無効：共有クリップボードリースの安全性審査が未完了です',
    biometricDisabled: '生体認証無効：実デバイス鍵保護は M2 です',
    lockedNotice: 'ロック済み。ローカルの機密状態を消去しました', busy: '安全に処理中…',
  },
};

export function translateVault(locale: SupportedLocale, key: string): string {
  return messages[locale][key] ?? messages['zh-CN'][key] ?? key;
}

export function vaultText(key: string): string {
  return translateVault(currentLocale.value, key);
}

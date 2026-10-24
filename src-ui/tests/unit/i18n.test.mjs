/**
 * 前端国际化 (i18n) 字典完整性、插值逻辑与 SVG Logo 结构独立单元测试
 * 存放于独立的 `src-ui/tests/unit/` 目录，与业务组件和静态配置文件严格分离。
 */

import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const localesDir = path.resolve(__dirname, '../../src/locales');
const publicLogoPath = path.resolve(__dirname, '../../public/logo.svg');
const assetLogoPath = path.resolve(__dirname, '../../src/assets/logo.svg');

const zhCN = JSON.parse(fs.readFileSync(path.join(localesDir, 'zh-CN.json'), 'utf8'));
const enUS = JSON.parse(fs.readFileSync(path.join(localesDir, 'en-US.json'), 'utf8'));
const jaJP = JSON.parse(fs.readFileSync(path.join(localesDir, 'ja-JP.json'), 'utf8'));

/** 递归收集嵌套字典中的所有点分叶子键路径 */
function collectLeafKeys(obj, prefix = '') {
  const keys = [];
  for (const [k, v] of Object.entries(obj)) {
    const fullKey = prefix ? `${prefix}.${k}` : k;
    if (v && typeof v === 'object' && !Array.isArray(v)) {
      keys.push(...collectLeafKeys(v, fullKey));
    } else {
      keys.push(fullKey);
    }
  }
  return keys.sort();
}

test('zh-CN, en-US, and ja-JP locale files have identical key schemas and non-empty translations', () => {
  const zhKeys = collectLeafKeys(zhCN);
  const enKeys = collectLeafKeys(enUS);
  const jaKeys = collectLeafKeys(jaJP);

  assert.deepEqual(enKeys, zhKeys, 'en-US keys must match zh-CN keys exactly');
  assert.deepEqual(jaKeys, zhKeys, 'ja-JP keys must match zh-CN keys exactly');
  assert.ok(zhKeys.length >= 90, `Expected at least 90 translation keys, got ${zhKeys.length}`);
});

test('high-fidelity SVG product logo exists in both public and assets directories and contains valid vector geometry', () => {
  assert.ok(fs.existsSync(publicLogoPath), 'public/logo.svg must exist');
  assert.ok(fs.existsSync(assetLogoPath), 'src/assets/logo.svg must exist');

  const svgContent = fs.readFileSync(publicLogoPath, 'utf8');
  assert.ok(svgContent.includes('viewBox="0 0 1024 1024"'), 'SVG must have 1024x1024 viewBox');
  assert.ok(svgContent.includes('#0084FF'), 'SVG must contain Doraemon sky blue background');
  assert.ok(svgContent.includes('#FFE042'), 'SVG must contain Take-copter golden yellow');
  assert.ok(!svgContent.includes('<image'), 'SVG must be pure vector geometry without raster <image> tags');
});

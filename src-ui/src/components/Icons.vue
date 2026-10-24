<template>
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    :stroke-width="strokeWidth"
    stroke-linecap="round"
    stroke-linejoin="round"
    class="macos-icon"
  >
    <!-- 剪贴板图标 -->
    <g v-if="name === 'clipboard'">
      <rect x="8" y="2" width="8" height="4" rx="1" ry="1" />
      <path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2" />
      <path d="M9 12h6" />
      <path d="M9 16h6" />
    </g>

    <!-- 便笺图标 -->
    <g v-else-if="name === 'sticky'">
      <path d="M15.5 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V8.5L15.5 3z" />
      <path d="M15 3v6h6" />
      <path d="M9 13h6" />
      <path d="M9 17h4" />
    </g>

    <!-- 图钉图标 (未固定) -->
    <g v-else-if="name === 'pin'">
      <path d="M16 3l5 5-2 2-3-3-4 4 1 5-2 2-4-4-4 4-2-2 4-4-4-4 2-2 5 1 4-4-3-3 2-2z" />
      <line x1="7" y1="17" x2="3" y2="21" />
    </g>

    <!-- 图钉图标 (已固定) -->
    <g v-else-if="name === 'pin-fill'">
      <path d="M16 3l5 5-2 2-3-3-4 4 1 5-2 2-4-4-4 4-2-2 4-4-4-4 2-2 5 1 4-4-3-3 2-2z" fill="currentColor" fill-opacity="0.2" />
      <line x1="7" y1="17" x2="3" y2="21" />
    </g>

    <!-- 主控台仪表盘图标 -->
    <g v-else-if="name === 'dashboard'">
      <rect x="3" y="3" width="7" height="9" rx="1" />
      <rect x="14" y="3" width="7" height="5" rx="1" />
      <rect x="14" y="12" width="7" height="9" rx="1" />
      <rect x="3" y="16" width="7" height="5" rx="1" />
    </g>

    <!-- 搜索图标 -->
    <g v-else-if="name === 'search'">
      <circle cx="11" cy="11" r="8" />
      <line x1="21" y1="21" x2="16.65" y2="16.65" />
    </g>

    <!-- 关闭/叉号图标 -->
    <g v-else-if="name === 'close'">
      <line x1="18" y1="6" x2="6" y2="18" />
      <line x1="6" y1="6" x2="18" y2="18" />
    </g>

    <!-- 垃圾桶/删除图标 -->
    <g v-else-if="name === 'trash'">
      <polyline points="3 6 5 6 21 6" />
      <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
    </g>

    <!-- 导出/下载图标 -->
    <g v-else-if="name === 'download'">
      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
      <polyline points="7 10 12 15 17 10" />
      <line x1="12" y1="15" x2="12" y2="3" />
    </g>

    <!-- 检查/对勾图标 -->
    <g v-else-if="name === 'check'">
      <polyline points="20 6 9 17 4 12" />
    </g>

    <!-- 图片照片图标 -->
    <g v-else-if="name === 'photo'">
      <rect x="3" y="3" width="18" height="18" rx="2" ry="2" />
      <circle cx="8.5" cy="8.5" r="1.5" />
      <polyline points="21 15 16 10 5 21" />
    </g>

    <!-- 文字排版图标 -->
    <g v-else-if="name === 'text'">
      <polyline points="4 7 4 4 20 4 20 7" />
      <line x1="9" y1="20" x2="15" y2="20" />
      <line x1="12" y1="4" x2="12" y2="20" />
    </g>

    <!-- 准星拾取图标 -->
    <g v-else-if="name === 'crosshair'">
      <circle cx="12" cy="12" r="10" />
      <line x1="22" y1="12" x2="18" y2="12" />
      <line x1="6" y1="12" x2="2" y2="12" />
      <line x1="12" y1="6" x2="12" y2="2" />
      <line x1="12" y1="22" x2="12" y2="18" />
    </g>

    <!-- 调色盘图标 -->
    <g v-else-if="name === 'palette'">
      <circle cx="13.5" cy="6.5" r=".5" fill="currentColor" />
      <circle cx="17.5" cy="10.5" r=".5" fill="currentColor" />
      <circle cx="8.5" cy="7.5" r=".5" fill="currentColor" />
      <circle cx="6.5" cy="12.5" r=".5" fill="currentColor" />
      <path d="M12 2C6.5 2 2 6.5 2 12s4.5 10 10 10c.926 0 1.648-.746 1.648-1.688 0-.437-.18-.835-.437-1.125-.29-.289-.438-.652-.438-1.125a1.64 1.64 0 0 1 1.668-1.668h1.996c3.051 0 5.563-2.512 5.563-5.563C22 6.5 17.5 2 12 2z" />
    </g>


    <!-- 终端/脚本图标 -->
    <g v-else-if="name === 'terminal'">
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <polyline points="7 9 10 12 7 15" />
      <line x1="12" y1="15" x2="17" y2="15" />
    </g>

    <!-- 运行图标 -->
    <g v-else-if="name === 'play'">
      <polygon points="6 4 20 12 6 20 6 4" />
    </g>

    <!-- 停止图标 -->
    <g v-else-if="name === 'stop'">
      <rect x="6" y="6" width="12" height="12" rx="1" />
    </g>

    <!-- 退出/电源图标 -->
    <g v-else-if="name === 'power'">
      <path d="M18.36 6.64a9 9 0 1 1-12.73 0" />
      <line x1="12" y1="2" x2="12" y2="12" />
    </g>

    <!-- 刷新图标 -->
    <g v-else-if="name === 'refresh'">
      <polyline points="23 4 23 10 17 10" />
      <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" />
    </g>

    <!-- 浅色模式/太阳图标 -->
    <g v-else-if="name === 'sun'">
      <circle cx="12" cy="12" r="5" />
      <line x1="12" y1="1" x2="12" y2="3" />
      <line x1="12" y1="21" x2="12" y2="23" />
      <line x1="4.22" y1="4.22" x2="5.64" y2="5.64" />
      <line x1="18.36" y1="18.36" x2="19.78" y2="19.78" />
      <line x1="1" y1="12" x2="3" y2="12" />
      <line x1="21" y1="12" x2="23" y2="12" />
      <line x1="4.22" y1="19.78" x2="5.64" y2="18.36" />
      <line x1="18.36" y1="5.64" x2="19.78" y2="4.22" />
    </g>

    <!-- 深色模式/月亮图标 -->
    <g v-else-if="name === 'moon'">
      <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
    </g>

    <!-- 跟随系统/显示器桌面图标 -->
    <g v-else-if="name === 'desktop'">
      <rect x="2" y="3" width="20" height="14" rx="2" ry="2" />
      <line x1="8" y1="21" x2="16" y2="21" />
      <line x1="12" y1="17" x2="12" y2="21" />
    </g>

    <!-- 多语言地球仪图标 -->
    <g v-else-if="name === 'globe'">
      <circle cx="12" cy="12" r="10" />
      <line x1="2" y1="12" x2="22" y2="12" />
      <path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z" />
    </g>

    <!-- 画笔图标 -->
    <g v-else-if="name === 'brush' || name === 'pen'">
      <path d="M12 19l7-7 3 3-7 7-3-3z" />
      <path d="M18 13l-1.5-7.5L2 2l3.5 14.5L13 18l5-5z" />
      <path d="M2 2l7.586 7.586" />
      <circle cx="11" cy="11" r="2" />
    </g>

    <!-- 荧光笔图标 -->
    <g v-else-if="name === 'highlighter'">
      <path d="M14 2l4 4-8 8H6v-4l8-8z" />
      <path d="M18 14l2 2-3 3-2-2" />
      <line x1="2" y1="22" x2="22" y2="22" />
    </g>

    <!-- 激光笔图标 -->
    <g v-else-if="name === 'laser'">
      <circle cx="12" cy="12" r="3" />
      <line x1="12" y1="2" x2="12" y2="6" />
      <line x1="12" y1="18" x2="12" y2="22" />
      <line x1="2" y1="12" x2="6" y2="12" />
      <line x1="18" y1="12" x2="22" y2="12" />
    </g>

    <!-- 直线图标 -->
    <g v-else-if="name === 'line'">
      <line x1="4" y1="20" x2="20" y2="4" />
      <circle cx="4" cy="20" r="1.5" />
      <circle cx="20" cy="4" r="1.5" />
    </g>

    <!-- 矩形图标 -->
    <g v-else-if="name === 'rect'">
      <rect x="3" y="3" width="18" height="18" rx="2" ry="2" />
    </g>

    <!-- 箭头图标 -->
    <g v-else-if="name === 'arrow'">
      <line x1="5" y1="19" x2="19" y2="5" />
      <polyline points="9 5 19 5 19 15" />
    </g>

    <!-- 橡皮擦图标 -->
    <g v-else-if="name === 'eraser'">
      <path d="M20 20H7L3 16C2 15 2 13 3 12L13 2C14 1 16 1 17 2L21 6C22 7 22 9 21 10L14 17" />
      <line x1="6" y1="13" x2="10" y2="17" />
    </g>

    <!-- 撤销图标 -->
    <g v-else-if="name === 'undo'">
      <polyline points="1 4 1 10 7 10" />
      <path d="M3.51 15a9 9 0 1 0 2.13-9.36L1 10" />
    </g>

    <!-- 重做图标 -->
    <g v-else-if="name === 'redo'">
      <polyline points="23 4 23 10 17 10" />
      <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" />
    </g>

    <!-- 鼠标/穿透操作图标 -->
    <g v-else-if="name === 'cursor' || name === 'mouse-pointer'">
      <path d="M3 3l7 18 3-7 7-3L3 3z" />
    </g>

    <!-- 相机/快照保存图标 -->
    <g v-else-if="name === 'camera'">
      <path d="M23 19a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4l2-3h6l2 3h4a2 2 0 0 1 2 2z" />
      <circle cx="12" cy="13" r="4" />
    </g>

    <!-- 复制图标 -->
    <g v-else-if="name === 'copy'">
      <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
      <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
    </g>

    <!-- 默认点点点 -->
    <g v-else>
      <circle cx="12" cy="12" r="1" />
      <circle cx="19" cy="12" r="1" />
      <circle cx="5" cy="12" r="1" />
    </g>
  </svg>
</template>

<script setup lang="ts">
/**
 * 苹果原生 SF Symbols 风格矢量图标组件
 * 替代所有粗糙 Emoji 字符，呈现精致克制的现代桌面风格
 */
withDefaults(
  defineProps<{
    name: string;
    size?: number | string;
    strokeWidth?: number | string;
  }>(),
  {
    size: 14,
    strokeWidth: 1.75,
  }
);
</script>

<style scoped>
.macos-icon {
  display: inline-block;
  vertical-align: middle;
  flex-shrink: 0;
}
</style>

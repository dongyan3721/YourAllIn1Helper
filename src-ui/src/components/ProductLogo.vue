<template>
  <!--
    产品官方高保真矢量 Logo 组件（哆啦A梦四维百宝袋与竹蜻蜓旋翼道具）
    支持任意像素无损缩放，用于主控台标题栏、剪贴板弹窗及关于标识
  -->
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 1024 1024"
    xmlns="http://www.w3.org/2000/svg"
    class="product-logo-svg"
    role="img"
    aria-label="All-in-One Toolbox Logo"
  >
    <defs>
      <clipPath :id="`${uid}-app-clip`">
        <rect x="0" y="0" width="1024" height="1024" rx="220" ry="220" />
      </clipPath>
      <clipPath :id="`${uid}-pocket-clip`">
        <path d="M 180 380 L 840 380 L 840 600 Q 506 620 180 600 Z" />
      </clipPath>
      <clipPath :id="`${uid}-flash-clip`">
        <path d="M 255 516 L 358 436 L 424 558 L 298 576 Z" />
      </clipPath>
    </defs>

    <g :clip-path="`url(#${uid}-app-clip)`">
      <!-- 1. 哆啦A梦天蓝背景 -->
      <rect x="0" y="0" width="1024" height="1024" fill="#0084FF" />

      <!-- 2. 右侧三枚黄色动感闪光三角 -->
      <g fill="#FFE144">
        <polygon points="688,448 708,354 738,376" />
        <polygon points="752,445 836,342 862,380" />
        <polygon points="786,466 846,444 846,492" />
      </g>

      <!-- 3. 百宝袋口深蓝衬底 -->
      <path
        d="M 216 526 C 340 560, 672 560, 796 526 L 770 595 L 242 595 Z"
        fill="#002259"
      />

      <!-- 4. 百宝袋内收纳道具组 -->
      <g :clip-path="`url(#${uid}-pocket-clip)`">
        <!-- 竹蜻蜓袋内底座延伸段 -->
        <path
          d="M 468 548 L 496 472 L 584 494 L 560 566 Z"
          fill="#FFE042"
          stroke="#002259"
          stroke-width="22"
          stroke-linejoin="round"
        />
        <path d="M 526 482 L 584 494 L 560 566 L 508 556 Z" fill="#F5B82E" />

        <!-- 左侧红色缩小灯手电筒 -->
        <g>
          <g :clip-path="`url(#${uid}-flash-clip)`">
            <path d="M 250 516 L 362 432 L 428 560 L 296 580 Z" fill="#FF4B3E" />
            <path d="M 332 452 L 362 432 L 424 554 L 388 562 Z" fill="#FF766B" />
            <path d="M 346 462 L 360 452 L 384 498 L 368 508 Z" fill="#FFF6CC" opacity="0.85" />
          </g>
          <path
            d="M 255 516 L 358 436 L 392 496 L 404 510 L 420 556 L 298 576 Z"
            fill="none"
            stroke="#002259"
            stroke-width="22"
            stroke-linejoin="round"
          />
          <path
            d="M 318 570 C 352 558, 378 538, 396 504"
            fill="none"
            stroke="#002259"
            stroke-width="20"
            stroke-linecap="round"
          />
          <ellipse
            cx="306"
            cy="476"
            rx="64"
            ry="34"
            transform="rotate(-38 306 476)"
            fill="#FFF6CC"
            stroke="#002259"
            stroke-width="22"
          />
        </g>

        <!-- 中部银灰圆环仪表 -->
        <g>
          <circle cx="494" cy="576" r="82" fill="#D6DCE5" stroke="#002259" stroke-width="22" />
          <circle cx="494" cy="580" r="50" fill="#737A86" stroke="#002259" stroke-width="20" />
        </g>

        <!-- 右侧翠绿道具卡 -->
        <g>
          <path
            d="M 572 574 L 630 462 C 636 450, 648 446, 660 452 L 772 504 C 784 510, 788 522, 782 534 L 748 594 Z"
            fill="#39BF8A"
            stroke="#002259"
            stroke-width="22"
            stroke-linejoin="round"
          />
          <path
            d="M 616 578 L 652 510 L 724 542 L 696 592 Z"
            fill="#9CE2C3"
            stroke="#002259"
            stroke-width="18"
            stroke-linejoin="round"
          />
        </g>
      </g>

      <!-- 5. 白色四维百宝袋主体 -->
      <g>
        <path
          d="M 212 534
             C 212 516, 228 514, 246 522
             C 340 568, 424 584, 506 584
             C 588 584, 672 568, 766 522
             C 784 514, 800 516, 800 534
             C 808 724, 674 868, 506 868
             C 338 868, 204 724, 212 534 Z"
          fill="#FFFFFF"
        />
        <path
          d="M 226 640
             C 258 784, 376 846, 506 846
             C 636 846, 754 784, 786 640
             C 766 798, 646 866, 506 866
             C 366 866, 246 798, 226 640 Z"
          fill="#DDE3EC"
        />
        <path
          d="M 212 534
             C 212 516, 228 514, 246 522
             C 340 568, 424 584, 506 584
             C 588 584, 672 568, 766 522
             C 784 514, 800 516, 800 534
             C 808 724, 674 868, 506 868
             C 338 868, 204 724, 212 534 Z"
          fill="none"
          stroke="#002259"
          stroke-width="24"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
        <path
          d="M 262 606 C 286 786, 726 786, 750 606"
          fill="none"
          stroke="#002259"
          stroke-width="22"
          stroke-linecap="round"
        />
      </g>

      <!-- 6. 竹蜻蜓 / 旋翼 Gadget -->
      <g>
        <path d="M 594 334 L 648 242 L 684 264 L 630 356 Z" fill="#FFE042" />
        <path d="M 612 344 L 666 252 L 684 264 L 630 356 Z" fill="#F5B82E" />
        <path
          d="M 594 334 L 648 242 L 684 264 L 630 356 Z"
          fill="none"
          stroke="#002259"
          stroke-width="22"
          stroke-linejoin="round"
        />

        <!-- 左旋翼桨叶 -->
        <path
          d="M 648 214
             C 620 184, 574 162, 544 166
             C 526 168, 520 186, 530 202
             C 548 228, 602 252, 642 246 Z"
          fill="#FFE042"
        />
        <path
          d="M 530 198 C 554 222, 604 242, 642 238 L 642 248 C 600 254, 546 230, 528 202 Z"
          fill="#F5B82E"
        />
        <path d="M 568 188 L 600 202" fill="none" stroke="#FFFFFF" stroke-width="11" stroke-linecap="round" />
        <circle cx="552" cy="181" r="5.5" fill="#FFFFFF" />
        <path
          d="M 648 214
             C 620 184, 574 162, 544 166
             C 526 168, 520 186, 530 202
             C 548 228, 602 252, 642 246 Z"
          fill="none"
          stroke="#002259"
          stroke-width="22"
          stroke-linejoin="round"
        />

        <!-- 右旋翼桨叶 -->
        <path
          d="M 692 240
             C 732 246, 788 272, 808 300
             C 818 314, 808 332, 790 332
             C 758 332, 708 302, 682 270 Z"
          fill="#FFE042"
        />
        <path
          d="M 684 264 C 712 294, 760 322, 794 322 L 790 332 C 756 332, 706 302, 680 270 Z"
          fill="#F5B82E"
        />
        <path d="M 744 278 L 774 296" fill="none" stroke="#FFFFFF" stroke-width="11" stroke-linecap="round" />
        <circle cx="786" cy="304" r="5.5" fill="#FFFFFF" />
        <path
          d="M 692 240
             C 732 246, 788 272, 808 300
             C 818 314, 808 332, 790 332
             C 758 332, 708 302, 682 270 Z"
          fill="none"
          stroke="#002259"
          stroke-width="22"
          stroke-linejoin="round"
        />

        <!-- 中心圆球轴承 -->
        <circle cx="666" cy="238" r="33" fill="#FFE042" stroke="#002259" stroke-width="22" />
        <path d="M 684 218 A 24 24 0 0 1 648 254 A 24 24 0 0 0 684 218 Z" fill="#F5B82E" />
      </g>

      <!-- 7. 哆啦A梦蓝色手臂与白色圆手 -->
      <g>
        <path
          d="M 476 296
             C 412 254, 316 196, 232 188
             C 164 182, 138 228, 156 284
             C 180 356, 278 418, 404 426 Z"
          fill="#0084FF"
        />
        <path
          d="M 476 296 C 412 254, 316 196, 232 188 C 178 184, 148 210, 150 250"
          fill="none"
          stroke="#002259"
          stroke-width="22"
          stroke-linecap="round"
        />
        <path
          d="M 152 254 C 162 338, 266 416, 404 426"
          fill="none"
          stroke="#002259"
          stroke-width="22"
          stroke-linecap="round"
        />

        <path
          d="M 454 316
             C 484 282, 554 284, 594 318
             C 634 342, 654 384, 628 416
             C 626 456, 588 492, 526 490
             C 448 488, 396 438, 398 378
             C 398 344, 422 320, 454 316 Z"
          fill="#FFFFFF"
        />
        <path
          d="M 412 414
             C 438 462, 504 476, 566 468
             C 596 462, 616 442, 624 418
             C 618 462, 578 492, 522 490
             C 454 488, 412 454, 412 414 Z"
          fill="#DDE3EC"
        />
        <path
          d="M 454 316
             C 484 282, 554 284, 594 318
             C 634 342, 654 384, 628 416
             C 626 456, 588 492, 526 490
             C 448 488, 396 438, 398 378
             C 398 344, 422 320, 454 316 Z"
          fill="none"
          stroke="#002259"
          stroke-width="24"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
        <path
          d="M 454 316 C 446 330, 442 342, 442 352"
          fill="none"
          stroke="#002259"
          stroke-width="20"
          stroke-linecap="round"
        />
        <path
          d="M 608 398 C 618 404, 624 410, 628 418"
          fill="none"
          stroke="#002259"
          stroke-width="20"
          stroke-linecap="round"
        />
      </g>
    </g>
  </svg>
</template>

<script setup lang="ts">
/**
 * 产品官方矢量 Logo 组件
 * 使用独立随机前缀防止同一页面渲染多个 SVG 实例时 clipPath ID 冲突
 */
withDefaults(
  defineProps<{
    size?: number | string;
  }>(),
  {
    size: 20,
  }
);

const uid = `logo-${Math.random().toString(36).slice(2, 8)}`;
</script>

<style scoped>
.product-logo-svg {
  display: inline-block;
  vertical-align: middle;
  flex-shrink: 0;
  border-radius: 22%;
  box-shadow: 0 1px 3px rgba(0, 34, 89, 0.25);
}
</style>

<script setup>
import { hue } from '../lib/useBoard.js'
defineProps({ name: String, size: { type: String, default: 'size-9' } })
</script>

<template>
  <span :class="size" class="inline-grid shrink-0 place-items-center rounded-full text-sm font-semibold text-white"
    :style="{ background: `hsl(${hue(name)} 55% 42%)` }">
    {{ name.slice(0, 1).toUpperCase() }}
  </span>
</template>

<script setup lang="ts">
import { withBase } from 'vitepress'

defineProps<{ name: string; title: string }>()
</script>

<template>
  <figure class="demo-video">
    <video
      controls
      playsinline
      preload="none"
      width="1280"
      height="780"
      :aria-label="title"
      :poster="withBase(`/demos/${name}.png`)"
    >
      <source :src="withBase(`/demos/${name}.mp4`)" type="video/mp4" />
      <a :href="withBase(`/demos/${name}.mp4`)">Watch {{ title }}</a>
    </video>
    <figcaption>
      <a :href="withBase(`/demos/${name}.mp4`)" download>Download MP4</a>
      <span aria-hidden="true"> · </span>
      <a :href="withBase(`/demos/${name}.gif`)">View GIF</a>
    </figcaption>
  </figure>
</template>

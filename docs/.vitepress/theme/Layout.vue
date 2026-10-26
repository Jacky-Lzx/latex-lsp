<script setup lang="ts">
import { withBase } from 'vitepress'
import DefaultTheme from 'vitepress/theme'
</script>

<template>
  <DefaultTheme.Layout>
    <template #home-hero-info-before>
      <img
        class="hero-logo"
        :src="withBase('/favicon.svg')"
        alt=""
        width="128"
        height="128"
      />
    </template>
  </DefaultTheme.Layout>
</template>

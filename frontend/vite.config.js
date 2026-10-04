import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [
    svelte({
      onwarn(warning, defaultHandler) {
        // 演示项目的内联 label 写法不影响功能，忽略 a11y 关联提示
        if (warning.code === 'a11y-label-has-associated-control') return;
        defaultHandler?.(warning);
      }
    })
  ],
  build: {
    outDir: 'dist',
    emptyOutDir: true
  },
  server: {
    port: 5173,
    proxy: {
      '/api': 'http://127.0.0.1:8787'
    }
  }
});

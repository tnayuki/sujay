import { defineConfig } from 'vite';

// https://vitejs.dev/config
export default defineConfig({
  build: {
    rollupOptions: {
      external: [
        // Native modules
        'naudiodon2',
        'fluent-ffmpeg',
        // Dependencies that should not be bundled
        'is-online',
        // Node built-ins that should not be bundled
        'electron',
        'fs',
        'path',
        'os',
      ],
    },
  },
});

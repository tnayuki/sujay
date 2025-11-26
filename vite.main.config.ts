import { defineConfig } from 'vite';

// https://vitejs.dev/config
export default defineConfig({
  build: {
    rollupOptions: {
      external: [
        // Native modules
        'naudiodon2',
        // Dependencies that should not be bundled
        'is-online',
        // MCP SDK - external to avoid bundling
        '@modelcontextprotocol/sdk',
        /^@modelcontextprotocol\/sdk\/.*/,
        // Node built-ins that should not be bundled
        'electron',
        'fs',
        'path',
        'os',
      ],
    },
  },
});

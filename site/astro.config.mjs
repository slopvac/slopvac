import { readFileSync } from 'node:fs';
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// site and base come from docs/site.toml through the generated site/generated/site.json.
// guides lists the Guides sidebar entries (label and slug), empty without guides.
// DOCS_BASE overrides the base path, for example for a preview or a custom domain.
const { site, base, guides = [] } = JSON.parse(readFileSync(new URL('./generated/site.json', import.meta.url), 'utf8'));

export default defineConfig({
  site,
  base: process.env.DOCS_BASE ?? base,
  trailingSlash: 'always',
  integrations: [
    starlight({
      title: 'slopvac',
      // Static site: no framing, no plugins, no external fetches. Starlight and Pagefind
      // need inline scripts and styles, and Pagefind loads WebAssembly.
      head: [
        {
          tag: 'meta',
          attrs: {
            'http-equiv': 'Content-Security-Policy',
            content: [
              "default-src 'self'",
              "script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'",
              "style-src 'self' 'unsafe-inline'",
              "img-src 'self' data:",
              "font-src 'self'",
              "connect-src 'self'",
              "object-src 'none'",
              "base-uri 'self'",
              "form-action 'self'",
            ].join('; '),
          },
        },
      ],
      social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/slopvac/slopvac' }],
      sidebar: [
        ...(guides.length ? [{ label: 'Guides', items: guides }] : []),
        { label: 'Reference (v3)', items: [{ autogenerate: { directory: 'v3' } }] },
      ],
    }),
  ],
});

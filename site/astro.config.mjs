import { readFileSync } from 'node:fs';
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// site and base come from docs/site.toml through the generated site/generated/site.json.
// DOCS_BASE overrides the base path, for example for a preview or a custom domain.
const { site, base } = JSON.parse(readFileSync(new URL('./generated/site.json', import.meta.url), 'utf8'));

export default defineConfig({
  site,
  base: process.env.DOCS_BASE ?? base,
  trailingSlash: 'always',
  integrations: [
    starlight({
      title: 'slopvac',
      social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/srobroek/slopvac' }],
      sidebar: [
        { label: 'Reference (v3)', items: [{ autogenerate: { directory: 'v3' } }] },
      ],
    }),
  ],
});

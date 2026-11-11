import fs from 'node:fs';
import path from 'node:path';
import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import type { Plugin } from 'vite';
import { defineConfig } from 'vitest/config';

/** Licenses of packages bundled into the JavaScript, written by Vite's build.license (deleted after reading) */
const VITE_LICENSE_JSON = '.vite/license.json';
/** File collecting the licenses of bundled packages, served with the page (at the root of dist) */
const LICENSES_FILE = 'licenses.txt';
/** CSS that imports packages */
const CSS_ENTRY = 'src/index.css';

/** A package's license (same shape as Vite's build.license JSON) */
interface PackageLicense {
  name: string;
  version: string;
  identifier?: string;
  text?: string;
}

/** Names of packages imported with @import in CSS (relative paths excluded) */
function cssImportedPackages(css: string): string[] {
  return [...css.matchAll(/^@import\s+['"]([^'"./][^'"]*)['"]/gm)].map(
    ([, specifier]) => {
      const parts = specifier.split('/');
      return specifier.startsWith('@') ? parts.slice(0, 2).join('/') : parts[0];
    },
  );
}

/** License of a package in node_modules. Fails if there is no LICENSE file */
function packageLicense(root: string, name: string): PackageLicense {
  const dir = path.join(root, 'node_modules', name);
  const { version, license } = JSON.parse(
    fs.readFileSync(path.join(dir, 'package.json'), 'utf-8'),
  ) as { version: string; license: string };
  const file = fs
    .readdirSync(dir)
    .find((entry) => /^(licen[cs]e|copying)/i.test(entry));
  if (!file) throw new Error(`${name} に LICENSE ファイルが無い`);
  const text = fs.readFileSync(path.join(dir, file), 'utf-8').trim();
  return { name, version, identifier: license, text };
}

function licensesText(packages: PackageLicense[]): string {
  const rule = '='.repeat(72);
  const sections = packages.map(
    ({ name, version, identifier, text }) =>
      `${rule}\n${name} ${version}${identifier ? ` (${identifier})` : ''}\n${rule}\n\n${text ?? ''}\n`,
  );
  return [
    'Third-party software included in the Draw MCP Connector canvas page:',
    sections.join('\n'),
  ].join('\n\n');
}

/**
 * Writes the file of bundled package licenses (LICENSES_FILE). The top of the JavaScript points to this file.
 * Vite's build.license only writes packages bundled into JavaScript,
 * so packages bundled into CSS (Tailwind, fonts, etc.) are added
 */
function licensesFile(): Plugin {
  let root = '';
  return {
    name: 'licenses-file',
    apply: 'build',
    configResolved(config) {
      root = config.root;
    },
    writeBundle({ dir }) {
      if (!dir) return;
      const read = (file: string) =>
        fs.readFileSync(path.join(root, file), 'utf-8');
      const viteJson = path.join(dir, VITE_LICENSE_JSON);
      const bundled = JSON.parse(
        fs.readFileSync(viteJson, 'utf-8'),
      ) as PackageLicense[];
      fs.rmSync(path.dirname(viteJson), { recursive: true });
      const fromCss = cssImportedPackages(read(CSS_ENTRY)).map((name) =>
        packageLicense(root, name),
      );
      fs.writeFileSync(
        path.join(dir, LICENSES_FILE),
        licensesText([...bundled, ...fromCss]),
      );
    },
  };
}

// https://vite.dev/config/
// https://vitest.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss(), licensesFile()],
  build: {
    // Licenses of bundled packages (copyright and permission notices required by MIT, etc.). licensesFile reads and collects them
    license: { fileName: VITE_LICENSE_JSON },
    rolldownOptions: {
      output: {
        // Added after minification, so it is not removed. The way to point to the license file described in Vite's docs (build.license)
        postBanner: `/*! Third-party licenses: /${LICENSES_FILE} */`,
      },
    },
  },
  resolve: {
    // Use tsconfig paths (@/*) in Vite and Vitest too
    tsconfigPaths: true,
  },
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    coverage: {
      provider: 'v8',
    },
  },
  server: {
    // Match the PC app's default canvas page (CANVAS_URL in src/bridge/src/config.rs).
    // localhost may listen only on IPv6 (::1), so specify the address.
    // If the port is in use, moving to another port would break the connection, so stop instead
    host: '127.0.0.1',
    port: 5173,
    strictPort: true,
  },
});

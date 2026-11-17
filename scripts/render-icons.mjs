// Optional asset generation: expose sharp through NODE_PATH, then run this file.
// Normal Cargo builds use the checked-in PNG/ICO/ICNS and do not require Node.js.
import { createRequire } from 'node:module';
import { mkdir, readFile, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
const require = createRequire(import.meta.url);
const sharp = require('sharp');
const path = (name) => fileURLToPath(new URL(`../assets/${name}`, import.meta.url));
const transparent = { r: 0, g: 0, b: 0, alpha: 0 };
// The extracted raster is the source of truth. All platform assets are derivatives.
const source = await readFile(path('branding/superfile-source.png'));
const artwork = await sharp(source).trim({ background: transparent }).png().toBuffer();
async function square(edge, artworkEdge) {
  const inset = (edge - artworkEdge) / 2;
  return sharp(artwork).resize(artworkEdge, artworkEdge, { fit: 'contain', background: transparent })
    .extend({ top: inset, bottom: inset, left: inset, right: inset, background: transparent }).png().toBuffer();
}
const master = await square(1024, 924);
await writeFile(path('branding/superfile.png'), master);
await sharp(master).resize(168, 168).png().toFile(path('branding/superfile-welcome.png'));
// Keep the existing SVG consumer compatible with the new raster artwork.
await writeFile(path('branding/superfile.svg'), `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024"><title>Superfile</title><desc>Generated raster wrapper; edit superfile-source.png.</desc><image width="1024" height="1024" href="data:image/png;base64,${master.toString('base64')}"/></svg>\n`);

const frames = await Promise.all([16, 24, 32, 48, 64, 128, 256].map(async (edge) => ({
  edge, png: await sharp(master).resize(edge, edge).png().toBuffer(),
})));
const directory = Buffer.alloc(6 + frames.length * 16);
directory.writeUInt16LE(1, 2);
directory.writeUInt16LE(frames.length, 4);
let offset = directory.length;
for (const [index, frame] of frames.entries()) {
  const at = 6 + index * 16;
  directory[at] = frame.edge === 256 ? 0 : frame.edge;
  directory[at + 1] = directory[at];
  directory.writeUInt16LE(1, at + 4);
  directory.writeUInt16LE(32, at + 6);
  directory.writeUInt32LE(frame.png.length, at + 8);
  directory.writeUInt32LE(offset, at + 12);
  offset += frame.png.length;
}
await mkdir(path('windows'), { recursive: true });
await writeFile(path('windows/Superfile.ico'), Buffer.concat([directory, ...frames.map((frame) => frame.png)]));

if (process.platform === 'darwin') {
  const temporary = await mkdtemp(join(tmpdir(), 'superfile-icons-'));
  try {
    const iconset = join(temporary, 'Superfile.iconset');
    await mkdir(iconset);
    const dock = await square(1024, 824);
    for (const edge of [16, 32, 128, 256, 512]) {
      for (const density of [1, 2]) {
        const suffix = density === 2 ? '@2x' : '';
        await sharp(dock).resize(edge * density, edge * density).png()
          .toFile(join(iconset, `icon_${edge}x${edge}${suffix}.png`));
      }
    }
    execFileSync('iconutil', ['-c', 'icns', iconset, '-o', path('macos/Superfile.icns')]);
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}
console.log('Generated Superfile PNG, SVG, Windows ICO, and macOS ICNS assets.');

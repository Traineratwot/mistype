#!/usr/bin/env node
/**
 * Deterministic HTML → PNG frames → MP4 (Mode B from html-to-video-pipeline).
 * Seeks window.__seek(t) per frame, screenshots, encodes with ffmpeg.
 *
 * Usage:
 *   node tools/render-demo.mjs [--fps 30] [--width 1920] [--height 1080] [--out demo/mistype-demo.mp4]
 */
import { chromium } from 'playwright';
import { mkdir, rm, writeFile } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const htmlPath = path.join(root, 'demo', 'mistype-demo.html');

function arg(name, dflt) {
  const i = process.argv.indexOf('--' + name);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : dflt;
}

const FPS = Number(arg('fps', '30'));
const WIDTH = Number(arg('width', '1920'));
const HEIGHT = Number(arg('height', '1080'));
const outMp4 = path.resolve(root, arg('out', 'demo/mistype-demo.mp4'));
const framesDir = path.join(root, 'demo', '.frames');

function run(cmd, args) {
  return new Promise((resolve, reject) => {
    const p = spawn(cmd, args, { stdio: 'inherit' });
    p.on('error', reject);
    p.on('exit', (code) =>
      code === 0 ? resolve() : reject(new Error(`${cmd} exited ${code}`))
    );
  });
}

async function main() {
  await rm(framesDir, { recursive: true, force: true });
  await mkdir(framesDir, { recursive: true });

  const browser = await chromium.launch({
    headless: true,
    executablePath: '/usr/bin/google-chrome-stable',
    args: ['--no-sandbox', '--disable-dev-shm-usage', '--font-render-hinting=none'],
  });

  const context = await browser.newContext({
    viewport: { width: WIDTH, height: HEIGHT },
    deviceScaleFactor: 1,
  });
  const page = await context.newPage();

  // file:// is fine here — no fetch() in the template
  await page.goto('file://' + htmlPath, { waitUntil: 'domcontentloaded' });

  // Fonts: local only (no CDN). Still wait for document.fonts so metrics settle.
  await page.evaluate(async () => {
    try {
      await Promise.all([...document.fonts].map((f) => f.load().catch(() => {})));
      await document.fonts.ready;
    } catch {}
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  });

  const duration = await page.evaluate(() => window.__DURATION || 34);
  const totalFrames = Math.ceil(duration * FPS);
  console.log(`Seeking ${totalFrames} frames @ ${FPS}fps (${duration}s) …`);

  for (let i = 0; i < totalFrames; i++) {
    const t = i / FPS;
    await page.evaluate((tt) => window.__seek(tt), t);
    // one rAF so layout/paint apply before screenshot
    await page.evaluate(
      () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)))
    );
    const file = path.join(framesDir, String(i).padStart(5, '0') + '.png');
    await page.screenshot({ path: file, type: 'png' });
    if (i % 30 === 0) console.log(`  frame ${i}/${totalFrames} t=${t.toFixed(2)}`);
  }

  await browser.close();

  console.log('Encoding MP4 …');
  await run('ffmpeg', [
    '-y',
    '-framerate', String(FPS),
    '-i', path.join(framesDir, '%05d.png'),
    '-c:v', 'libx264',
    '-pix_fmt', 'yuv420p',
    '-preset', 'medium',
    '-crf', '18',
    '-movflags', '+faststart',
    outMp4,
  ]);

  // verify
  await run('ffprobe', [
    '-v', 'error',
    '-show_entries', 'format=duration',
    '-of', 'csv=p=0',
    outMp4,
  ]);

  // sample frames for sanity
  const samples = path.join(root, 'demo', '.samples');
  await mkdir(samples, { recursive: true });
  for (const t of ['0.4', '5', '12', '18', '25', '31']) {
    await run('ffmpeg', [
      '-y', '-ss', t, '-i', outMp4, '-frames:v', '1',
      path.join(samples, `t${t}.png`),
    ]);
  }

  // write a simple path note
  await writeFile(
    path.join(root, 'demo', 'RENDER.txt'),
    `Output: ${outMp4}\nDuration target: ${duration}s @ ${FPS}fps\nFrames: ${totalFrames}\n`
  );
  console.log('Done:', outMp4);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});

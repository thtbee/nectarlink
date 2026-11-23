// SPDX-License-Identifier: GPL-3.0-or-later
import {createHash} from 'node:crypto';
import {mkdir} from 'node:fs/promises';
import {siteMarker} from './src/site-protocol.js';

const {outputs} = await Bun.build({entrypoints: ['src/site.js'], target: 'browser', minify: true});
if (outputs.length !== 1) throw Error('Expected one self-contained receiver bundle');
const notices = await Promise.all(['@noble/hashes', '@noble/ciphers'].map(async (packageName) =>
  `/*! ${packageName}\n${(await Bun.file(`node_modules/${packageName}/LICENSE`).text()).trim()}\n*/\n`));
const script = notices.join('') + await outputs[0].text();
const integrity = createHash('sha256').update(script).digest('base64');
const page = `<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="referrer" content="no-referrer">
<meta ${siteMarker}>
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'self'; style-src 'unsafe-inline'; connect-src 'self' http: wss:; base-uri 'none'; form-action 'none'">
<title></title>
<script src="./receive.js?v=3" integrity="sha256-${integrity}" crossorigin="anonymous" defer></script>
</html>
`;
await mkdir('site', {recursive: true});
await Promise.all([Bun.write('site/index.html', page), Bun.write('site/receive.js', script)]);

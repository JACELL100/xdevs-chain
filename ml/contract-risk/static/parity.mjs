// Checks that core.js reproduces the Python scores: node parity.mjs <exported dir> <parity.json>
import fs from 'node:fs';
import path from 'node:path';
import { score } from './core.js';

const [dir, file] = process.argv.slice(2);
const trees = JSON.parse(fs.readFileSync(path.join(dir, 'trees.json'), 'utf8'));
const config = JSON.parse(fs.readFileSync(path.join(dir, 'config.json'), 'utf8'));
const buf = fs.readFileSync(path.join(dir, 'linear.bin'));
const coef = new Float32Array(buf.buffer, buf.byteOffset, buf.byteLength / 4);
const model = { trees, coef, intercept: config.intercept, config };
const { codes, scores } = JSON.parse(fs.readFileSync(file, 'utf8'));
let worst = 0;
codes.forEach((c, i) => { worst = Math.max(worst, Math.abs(score(model, c) - scores[i])); });
console.log(`parity: ${codes.length} contracts, max |js - python| = ${worst.toExponential(2)}`);
if (!(worst < 1e-3)) { console.error('PARITY FAILED'); process.exit(1); }

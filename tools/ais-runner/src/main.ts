#!/usr/bin/env node
import { run } from './runner/router.js';

await run(process.argv.slice(2));

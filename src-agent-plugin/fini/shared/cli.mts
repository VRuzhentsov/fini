#!/usr/bin/env node
import { execute } from './core.mts';
import { serve } from './transport.mts';
await serve(execute);

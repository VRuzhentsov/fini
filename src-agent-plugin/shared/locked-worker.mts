// Internal entry point: exec'd by flock --no-fork; never call directly.
import { normalizeRequest } from './core.mts';
import { transaction } from './store.mts';
import { serve } from './transport.mts';
await serve(request => transaction(normalizeRequest(request)));

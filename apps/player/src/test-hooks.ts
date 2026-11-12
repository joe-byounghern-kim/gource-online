// SPDX-License-Identifier: GPL-3.0-or-later

// The engine for the browser specs: `window.__gource`, and `?fixed-dt=<seconds>`, where the simulation advances only
// through `step`. Exists only in builds made with VITE_E2E_HOOKS=1; a release build has neither.

import type { EngineStatus, GourceApp } from '@gource-online/viewer';

export interface GourceTestHook {
  app: GourceApp;
  status(): EngineStatus;
  step(ticks: number): void;
  setFixedDt(seconds: number): void;
}

declare global {
  interface Window {
    __gource?: GourceTestHook;
  }
}

const HOOKS = import.meta.env.VITE_E2E_HOOKS === '1';

export function exposeEngine(app: GourceApp): void {
  if (!HOOKS) return;
  const fixedDt = Number(new URLSearchParams(window.location.search).get('fixed-dt'));
  if (Number.isFinite(fixedDt) && fixedDt > 0) app.set_fixed_dt(fixedDt);
  window.__gource = {
    app,
    status: () => JSON.parse(app.status()) as EngineStatus,
    step: (ticks) => app.step(ticks),
    setFixedDt: (seconds) => app.set_fixed_dt(seconds),
  };
}

export function clearTestHook(): void {
  if (HOOKS) delete window.__gource;
}

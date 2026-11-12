// SPDX-License-Identifier: GPL-3.0-or-later

/** How long a run waits for the label font before it starts with the fallback face. */
export const FONT_TIMEOUT_MS = 4000;

const loads = new Map<string, Promise<void>>();

/**
 * Loads FreeSans as "GourceSans": the engine's glyph atlas rasterises with
 * it. Resolves after `timeoutMs` at the latest; labels then use the
 * fallback face (a late font still gets added for the next run).
 */
export function loadGourceFont(url: string, timeoutMs = FONT_TIMEOUT_MS): Promise<void> {
  let load = loads.get(url);
  if (!load) {
    load = (async () => {
      try {
        const face = new FontFace('GourceSans', `url(${url})`);
        await face.load();
        document.fonts.add(face);
      } catch (err) {
        console.warn('Could not load the GourceSans font; labels fall back to sans-serif.', err);
      }
    })();
    loads.set(url, load);
  }
  let timer: number | undefined;
  const timeout = new Promise<void>((resolve) => {
    timer = window.setTimeout(() => {
      console.warn(`The GourceSans font did not load within ${timeoutMs / 1000} s; starting with sans-serif.`);
      resolve();
    }, timeoutMs);
  });
  void load.then(() => window.clearTimeout(timer));
  return Promise.race([load, timeout]);
}

let webgl2: boolean | undefined;

/** Whether this browser can create a WebGL2 context (probed once, then released). */
export function webgl2Available(): boolean {
  if (webgl2 === undefined) {
    try {
      const gl = document.createElement('canvas').getContext('webgl2');
      webgl2 = gl !== null;
      gl?.getExtension('WEBGL_lose_context')?.loseContext();
    } catch {
      webgl2 = false;
    }
  }
  return webgl2;
}

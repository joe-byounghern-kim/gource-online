// SPDX-License-Identifier: GPL-3.0-or-later

// The start screen: what the player shows with no log to play. Opening a local
// file comes first; beside it, how to make one (Gource's git command, or the
// one-step `npx gource-online`), a pointer to the hosted site, and what this
// player is. It is plain DOM: no style attribute, script or request of its
// own, so the player's CSP holds.

import { shellLogCommand } from '@gource-online/protocol';
import { npxCommand, PUBLIC_REPO_URL } from './build-info';
import { h, icon } from './dom';

/** The file the git command writes. The file picker offers `.gitlog`. */
export const GIT_LOG_FILE = 'history.gitlog';

export interface StartScreenOptions {
  /** The hosted site's origin (build-info.ts `hostedSiteUrl`). */
  site: string;
  /** Embedded in a page: that page is the hosted site, so the pointer to it is left out. */
  embedded: boolean;
  /** The licence, source and notices line. */
  licence: HTMLElement;
  /** Opens the file picker. */
  open(): void;
}

export interface StartScreen {
  el: HTMLElement;
  /** The primary action, which takes focus back from a log. */
  openButton: HTMLButtonElement;
}

/** A link that opens in a new tab and says so. */
function outLink(href: string, className: string, ...children: Array<Node | string>): HTMLAnchorElement {
  return h(
    'a',
    { href, class: className, target: '_blank', rel: 'noopener noreferrer' },
    ...children,
    h('span.visually-hidden', {}, ' (opens in a new tab)'),
    icon('external'),
  );
}

function selectText(node: Node): void {
  const range = document.createRange();
  range.selectNodeContents(node);
  const selection = window.getSelection();
  selection?.removeAllRanges();
  selection?.addRange(range);
}

/**
 * Copies `text`. Without the clipboard API (an insecure origin, a frame without clipboard-write, a refusal), the
 * command is selected instead, for Ctrl+C.
 */
async function copyCommand(text: string, code: HTMLElement, button: HTMLButtonElement, status: HTMLElement): Promise<void> {
  try {
    if (!navigator.clipboard?.writeText) throw new Error('no clipboard');
    await navigator.clipboard.writeText(text);
    button.classList.add('is-done');
    status.textContent = 'Copied';
  } catch {
    selectText(code);
    status.textContent = 'Selected: press Ctrl+C (⌘C on a Mac) to copy';
  }
  window.setTimeout(() => {
    button.classList.remove('is-done');
    status.textContent = '';
  }, 4000);
}

/** A command to paste in a terminal, with a Copy button. */
function command(name: string, caption: string, text: string): HTMLElement {
  const code = h('code', {}, text);
  const status = h('span.start-copy-status', { role: 'status' });
  const button = h('button.btn.btn-small.start-copy', { type: 'button', 'aria-label': `Copy the ${name}` }, icon('copy'), 'Copy');
  button.addEventListener('click', () => void copyCommand(text, code, button, status));
  return h(
    'div.start-command',
    {},
    h('div.start-command-head', {}, h('span.start-command-caption', {}, icon('terminal'), caption), status, button),
    h('pre.start-command-text', { tabindex: '0', 'aria-label': name }, code),
  );
}

/** Gource's look in miniature: a directory node, its files in Gource's action colours. */
export function mark(): SVGSVGElement {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', '0 0 32 32');
  svg.setAttribute('class', 'player-mark');
  svg.setAttribute('aria-hidden', 'true');
  svg.innerHTML =
    '<path class="mark-edge" d="M16 17L7 8M16 17L26 9M16 17L25 26M16 17L6 24"/>' +
    '<circle class="mark-add" cx="7" cy="8" r="2.6"/><circle class="mark-modify" cx="26" cy="9" r="2.2"/>' +
    '<circle class="mark-add" cx="25" cy="26" r="2"/><circle class="mark-delete" cx="6" cy="24" r="1.8"/>' +
    '<circle class="mark-node" cx="16" cy="17" r="3.4"/>';
  return svg;
}

export function startScreen(options: StartScreenOptions): StartScreen {
  const openButton = h('button.btn.btn-primary.start-open', { type: 'button' }, icon('upload'), 'Open a log file');
  openButton.addEventListener('click', () => options.open());
  const siteHost = new URL(options.site).host;

  const drop = h(
    'section.start-drop',
    { 'aria-labelledby': 'start-drop-title' },
    h('h3.visually-hidden', { id: 'start-drop-title' }, 'Open a log file'),
    h('div.start-drop-icon', {}, icon('file')),
    openButton,
    h('p.start-drop-hint', {}, 'or drop it anywhere on this page'),
    h('p.start-drop-formats', {}, 'A Gource custom log, or the output of Gource\'s git log command.'),
    h('p.start-privacy', {}, icon('lock'), 'The file is read in this browser. Nothing is uploaded.'),
  );

  const howto = h(
    'section.start-howto',
    { 'aria-labelledby': 'start-howto-title' },
    h('h3', { id: 'start-howto-title' }, 'No log file yet?'),
    h(
      'ol.start-steps',
      {},
      h(
        'li',
        {},
        h('p', {}, 'In a terminal, inside your repository, write its history to ', h('code', {}, GIT_LOG_FILE), ':'),
        command('git command', 'git', shellLogCommand(GIT_LOG_FILE)),
        h('p.start-note', {}, 'It holds commit times, authors and file paths: no file contents.'),
      ),
      h('li', {}, h('p', {}, 'Open ', h('code', {}, GIT_LOG_FILE), ' here, or drop it on this page.')),
    ),
    h(
      'div.start-oneshot',
      {},
      h('h4', {}, 'Or in one step, with Node.js'),
      h('p', {}, 'Run this inside your repository instead. It reads the history with git and opens it in a copy of this player served from your computer.'),
      command('npx command', 'Node.js 18.18 or later', npxCommand()),
    ),
  );

  const site = options.embedded
    ? null
    : h(
        'aside.start-site',
        { 'aria-label': siteHost },
        h('p', {}, h('strong', {}, 'A public repository? '), `On ${siteHost}, paste its GitHub or GitLab URL instead: no file to make, and more features.`),
        outLink(options.site, 'btn btn-small start-site-link', `Go to ${siteHost}`),
      );

  const about = h(
    'footer.start-about',
    {},
    h(
      'section',
      { 'aria-labelledby': 'start-about-gource' },
      h('h3', { id: 'start-about-gource' }, 'About Gource'),
      h(
        'p',
        {},
        'Gource is the software version control visualization created by Andrew Caudwell. This player is an independent port of it to the browser, in Rust and WebAssembly, and free software under the GPL-3.0-or-later like Gource. It is not made or endorsed by Gource\'s author.',
      ),
      h('p.start-links', {}, outLink('https://gource.io', 'start-link', 'gource.io'), outLink('https://github.com/acaudwell/Gource', 'start-link', 'Gource on GitHub')),
    ),
    h(
      'section',
      { 'aria-labelledby': 'start-about-source' },
      h('h3', { id: 'start-about-source' }, 'Open source'),
      h('p', {}, 'Gource Online is developed in the open. If you like it, a star on GitHub helps other people find it.'),
      h('p.start-links', {}, outLink(PUBLIC_REPO_URL, 'btn btn-small start-star', icon('star'), 'Star it on GitHub')),
    ),
    options.licence,
  );

  const el = h(
    'section.player-empty',
    { 'aria-labelledby': 'start-title' },
    h(
      'div.start-intro',
      {},
      h('h2', { id: 'start-title' }, 'Play a Gource log'),
      h('p.start-lead', {}, 'Watch a project\'s history grow: its files branch out as a tree, and each contributor flies to the files they change, commit by commit.'),
    ),
    h('div.start-grid', {}, drop, howto),
    site,
    about,
  );
  return { el, openButton };
}

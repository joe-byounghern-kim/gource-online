// SPDX-License-Identifier: GPL-3.0-or-later

// Panel section "Contributors & AI" (F1 section 4.4, AI-64) and the URL
// notices of AI-74. Controls: Show decorations (the inverse of
// hide=decorations), the built-in look (ai-style), the agent key, the AI
// commits filter (ai-filter, re-simulates), Reduce motion (decor-motion), the
// "My rules" box (user-style) with presets and check_style errors, and the
// "AI in this repository" summary from ai_stats(), read twice a second while
// the section is open. Data from the log reaches the DOM through textContent
// only.

import './ai.css';

import { h, icon } from '../dom';
import type { GourceApp } from '@gource-online/viewer';
import { parseWasmError } from '../errors';
import { formatCount } from '../format';
import { SHARE_URL_SOFT_LIMIT } from '../links';
import { OPTIONS_BY_NAME, SECTIONS, normaliseHide, normaliseValue, type ListDef, type SelectDef } from '@gource-online/viewer/options';
import { RULE_PRESETS, RULES_HELP } from './presets';
import { parseAiStats, type AiAgentStats, type AiStats } from './stats';
import type { AiStart } from './stored';
import { storeAiOption } from './stored';

export interface AiSectionHost {
  /** Applies one option (and mirrors it into the URL); returns an error message or null. */
  setOption(name: string, value: string): string | null;
  /** The length of the link to this view with these option values, or null when the page has no such link. */
  linkLength?(values: ReadonlyMap<string, string>): number | null;
}

/** Where the loaded log came from, for the "no attribution data" state (AI-29). */
export interface AiSource {
  /** The server's `ai_detect` (absent for `?log=` and local files). */
  aiDetect?: boolean | undefined;
  /** The whole log is in the engine. */
  finished: boolean;
}

const STATS_INTERVAL_MS = 500;
const RULES_DEBOUNCE_MS = 300;
const NO_DATA = 'This log has no AI attribution data (detection off or log predates it).';
const CAVEAT = 'From commit trailers; not proof, may be undercounted.';

interface CheckResult {
  ok: boolean;
  code?: string;
  line?: number;
  message?: string;
}

function parseCheck(json: string): CheckResult {
  try {
    const r = JSON.parse(json) as CheckResult;
    return typeof r.ok === 'boolean' ? r : { ok: true };
  } catch {
    return { ok: true };
  }
}

export class AiSection {
  readonly el: HTMLDetailsElement;
  /** Dismissible notices for the stage (AI-74). */
  readonly notices: HTMLElement;
  private readonly host: AiSectionHost;
  private readonly values: Map<string, string>;
  private engine: GourceApp | null = null;
  private source: AiSource = { finished: false };
  private lastStatsJson = '';
  private timer: number | undefined;
  private rulesTimer: number | undefined;

  private readonly showDeco: HTMLInputElement;
  private readonly look: HTMLSelectElement;
  private readonly agentKey: HTMLInputElement;
  private readonly filterRadios: HTMLInputElement[];
  private readonly motion: HTMLInputElement;
  private readonly preset: HTMLSelectElement;
  private readonly rules: HTMLTextAreaElement;
  private readonly rulesError: HTMLElement;
  private readonly lengthHint: HTMLElement;
  private readonly sectionError: HTMLElement;
  private readonly aiGroup: HTMLFieldSetElement;
  private readonly noData: HTMLElement;
  private readonly summary: HTMLElement;

  constructor(host: AiSectionHost, values: Map<string, string>, start: Pick<AiStart, 'styleFromUrl' | 'filterFromUrl'>) {
    this.host = host;
    this.values = values;

    // --- Show decorations, Reduce motion: apply with or without AI data ---
    this.showDeco = h('input.switch-input', { id: 'opt-show-decorations', type: 'checkbox', role: 'switch', 'data-ai-control': 'decorations' });
    this.showDeco.addEventListener('change', () => this.setDecorations(this.showDeco.checked));
    this.motion = h('input.switch-input', { id: 'opt-decor-motion', type: 'checkbox', role: 'switch', 'data-option': 'decor-motion' });
    this.motion.addEventListener('change', () => this.apply('decor-motion', this.motion.checked ? 'off' : 'full'));

    // --- AI-only controls, disabled without attribution data ---
    const lookDef = OPTIONS_BY_NAME.get('ai-style') as SelectDef;
    this.look = h('select.select-input', { id: 'opt-ai-style', 'data-option': 'ai-style' }, ...lookDef.choices.map((c) => h('option', { value: c.value }, c.label)));
    this.look.addEventListener('change', () => this.apply('ai-style', this.look.value));
    this.agentKey = h('input.switch-input', { id: 'opt-agent-key', type: 'checkbox', role: 'switch', 'data-option': 'agent-key' });
    this.agentKey.addEventListener('change', () => this.apply('agent-key', String(this.agentKey.checked)));
    const filterDef = OPTIONS_BY_NAME.get('ai-filter') as SelectDef;
    this.filterRadios = filterDef.choices.map((c) =>
      h('input', { type: 'radio', name: 'ai-filter', value: c.value, 'data-option': 'ai-filter' }),
    );
    for (const radio of this.filterRadios) {
      radio.addEventListener('change', () => {
        if (radio.checked) this.apply('ai-filter', radio.value);
      });
    }
    const filterGroup = h(
      'fieldset.ai-filter',
      { 'data-control': 'ai-filter' },
      h('legend.control-label', {}, 'AI commits'),
      h(
        'div.ai-radios',
        {},
        ...filterDef.choices.map((c, i) => h('label.ai-radio', {}, this.filterRadios[i]!, h('span', {}, c.label))),
      ),
      h('p.control-hint.ai-hint', {}, 'Re-simulates from the downloaded history.'),
    );
    this.noData = h('p.ai-no-data', { hidden: true }, NO_DATA);
    this.summary = h('div.ai-summary', { 'aria-live': 'off' });
    this.aiGroup = h(
      'fieldset.ai-group',
      {},
      h('legend.ai-vh', {}, 'AI attribution'),
      switchRow('opt-agent-key', 'Agent key', this.agentKey, 'agent-key'),
      filterGroup,
      h('div.control.control-select', { 'data-control': 'ai-style' }, h('div.control-row', {}, h('label.control-label', { for: 'opt-ai-style' }, 'Built-in look'), this.look)),
    );

    // --- My rules ---
    this.preset = h(
      'select.select-input.ai-preset',
      { id: 'opt-user-style-preset', 'aria-label': 'Rule preset' },
      h('option', { value: '' }, 'Preset…'),
      ...RULE_PRESETS.map((p) => h('option', { value: p.id }, p.label)),
    );
    this.preset.addEventListener('change', () => {
      const preset = RULE_PRESETS.find((p) => p.id === this.preset.value);
      if (!preset) return;
      this.rules.value = preset.rules;
      this.checkRules();
    });
    this.rules = h('textarea.list-input.ai-rules', {
      id: 'opt-user-style',
      rows: 4,
      spellcheck: 'false',
      autocomplete: 'off',
      placeholder: (OPTIONS_BY_NAME.get('user-style') as ListDef | undefined)?.placeholder ?? '',
      'data-option': 'user-style',
      'aria-describedby': 'opt-user-style-error opt-user-style-help',
    });
    this.rules.addEventListener('input', () => {
      window.clearTimeout(this.rulesTimer);
      this.rulesTimer = window.setTimeout(() => this.checkRules(), RULES_DEBOUNCE_MS);
      this.syncPreset();
    });
    this.rulesError = h('p.control-error.ai-rules-error', { id: 'opt-user-style-error', 'aria-live': 'polite' });
    this.lengthHint = h('p.ai-length-hint', { hidden: true, 'aria-live': 'polite' });
    this.sectionError = h('p.control-error', { 'aria-live': 'polite' });

    const rulesControl = h(
      'div.control.control-rules',
      { 'data-control': 'user-style' },
      h('div.ai-rules-head', {}, h('label.control-label', { for: 'opt-user-style' }, 'My rules'), this.preset),
      this.rules,
      this.rulesError,
      h('p.control-hint.ai-hint', { id: 'opt-user-style-help' }, RULES_HELP),
      this.lengthHint,
    );

    const title = SECTIONS.find((s) => s.id === 'style')?.title ?? 'Contributors & AI';
    this.el = h(
      'details.panel-section',
      { 'data-section': 'style' },
      h('summary', {}, title),
      h(
        'div.section-content',
        {},
        switchRow('opt-show-decorations', 'Show decorations', this.showDeco, 'decorations'),
        this.noData,
        this.aiGroup,
        switchRow('opt-decor-motion', 'Reduce motion', this.motion, 'decor-motion'),
        rulesControl,
        this.sectionError,
        h('section.ai-stats', { 'aria-labelledby': 'ai-stats-title' }, h('h3.ai-stats-title#ai-stats-title', {}, 'AI in this repository'), this.summary, h('p.ai-caveat', {}, CAVEAT)),
      ),
    );
    this.el.addEventListener('toggle', () => this.onToggle());

    this.notices = h('div.ai-notices', { role: 'status' });
    if (start.styleFromUrl) this.addNotice('style', 'This link applies custom styling.');
    if (start.filterFromUrl) this.addNotice('filter', 'This link filters AI commits.');

    for (const name of ['hide', 'ai-style', 'agent-key', 'ai-filter', 'decor-motion', 'user-style']) this.setValue(name, this.current(name));
  }

  attach(engine: GourceApp): void {
    this.engine = engine;
    for (const name of ['hide', 'ai-style', 'agent-key', 'ai-filter', 'decor-motion', 'user-style']) this.setValue(name, this.current(name));
    this.refresh();
  }

  setSource(source: AiSource): void {
    const changed = source.finished !== this.source.finished || source.aiDetect !== this.source.aiDetect;
    this.source = { ...source };
    if (changed) this.refresh();
  }

  /** Reflects a value changed elsewhere (URL, engine keys, the Hide chips, a rejected start option). */
  setValue(name: string, value: string): void {
    switch (name) {
      case 'hide':
        this.showDeco.checked = !normaliseHide(value.split(',')).includes('decorations');
        break;
      case 'ai-style':
        this.look.value = value || 'default';
        break;
      case 'agent-key':
        this.agentKey.checked = value === 'true';
        break;
      case 'ai-filter':
        for (const r of this.filterRadios) r.checked = r.value === (value || 'all');
        break;
      case 'decor-motion':
        this.motion.checked = value === 'off';
        break;
      case 'user-style':
        // Never rewrite the box under the viewer's cursor.
        if (document.activeElement !== this.rules) {
          this.rules.value = value;
          this.syncPreset();
          this.showRulesError(null);
        }
        break;
      default:
        break;
    }
  }

  /** An option the viewer changed was applied: reflect and remember it (US-15). */
  optionApplied(name: string, value: string): void {
    this.setValue(name, value);
    storeAiOption(name, value);
    if (name === 'user-style') this.updateLengthHint();
  }

  dispose(): void {
    window.clearInterval(this.timer);
    window.clearTimeout(this.rulesTimer);
    this.timer = undefined;
    this.engine = null;
  }

  // --- controls ---

  private current(name: string): string {
    return this.values.get(name) ?? OPTIONS_BY_NAME.get(name)?.default ?? '';
  }

  private apply(name: string, value: string): void {
    const err = this.host.setOption(name, value);
    this.sectionError.textContent = err === null ? '' : parseWasmError(err).text;
    // A rejected value puts the control back.
    if (err !== null) this.setValue(name, this.current(name));
  }

  private setDecorations(show: boolean): void {
    const items = new Set<string>(normaliseHide(this.current('hide').split(',')));
    if (show) items.delete('decorations');
    else items.add('decorations');
    this.apply('hide', normaliseHide(items).join(','));
  }

  /** Validates the rules box with check_style; applies it only when every line is valid (US-6). */
  private checkRules(): void {
    window.clearTimeout(this.rulesTimer);
    const engine = this.engine;
    if (!engine) return;
    const text = this.rules.value;
    let result: CheckResult;
    try {
      result = parseCheck(engine.check_style(text));
    } catch (e) {
      this.showRulesError(parseWasmError(e).text);
      return;
    }
    if (!result.ok) {
      const where = result.line ? `Line ${result.line}: ` : '';
      this.showRulesError(`${where}${result.code ?? 'style_error'}: ${result.message ?? ''}`.trim(), result.code);
      return;
    }
    const value = normaliseValue('user-style', text);
    if (value === normaliseValue('user-style', this.current('user-style'))) {
      this.showRulesError(null);
      return;
    }
    const err = this.host.setOption('user-style', value);
    if (err === null) {
      this.showRulesError(null);
      return;
    }
    const { code, text: message } = parseWasmError(err);
    this.showRulesError(code ? `${code}: ${message}` : message, code);
  }

  private showRulesError(message: string | null, code?: string): void {
    this.rulesError.textContent = message ?? '';
    if (code) this.rulesError.dataset.code = code;
    else delete this.rulesError.dataset.code;
    this.rules.closest('.control')?.classList.toggle('is-invalid', message !== null);
    if (message === null) this.rules.removeAttribute('aria-invalid');
    else this.rules.setAttribute('aria-invalid', 'true');
  }

  private syncPreset(): void {
    const value = normaliseValue('user-style', this.rules.value);
    const preset = RULE_PRESETS.find((p) => p.rules === value);
    this.preset.value = preset && value !== '' ? preset.id : '';
  }

  /** SHARE_URL_SOFT_LIMIT: the link still works, but some apps cut long links. */
  private updateLengthHint(): void {
    let length = 0;
    try {
      length = this.host.linkLength?.(this.values) ?? 0;
    } catch {
      length = 0;
    }
    const long = length > SHARE_URL_SOFT_LIMIT && this.current('user-style') !== '';
    this.lengthHint.hidden = !long;
    this.lengthHint.textContent = long
      ? `This style is long: the link to this view has ${formatCount(length)} characters, and some apps cut links over ${formatCount(SHARE_URL_SOFT_LIMIT)}.`
      : '';
  }

  // --- notices (AI-74) ---

  private addNotice(kind: string, text: string): void {
    const notice = h(
      'div.ai-notice',
      { 'data-notice': kind },
      h('span.ai-notice-text', {}, text),
      h('button.icon-btn.ai-notice-close', { type: 'button', 'aria-label': 'Dismiss notice', onclick: () => notice.remove() }, icon('close')),
    );
    this.notices.append(notice);
  }

  // --- AI in this repository ---

  private onToggle(): void {
    window.clearInterval(this.timer);
    this.timer = undefined;
    if (!this.el.open) return;
    this.refresh();
    this.updateLengthHint();
    this.timer = window.setInterval(() => {
      // Twice a second, and only while the section is open in an open panel.
      if (this.el.closest('.panel')?.classList.contains('is-open') === false) return;
      this.refresh();
    }, STATS_INTERVAL_MS);
  }

  private refresh(): void {
    const engine = this.engine;
    if (!engine) return;
    let json: string;
    try {
      json = engine.ai_stats();
    } catch {
      return;
    }
    const stats = parseAiStats(json);
    const noData = this.source.aiDetect === false || (stats !== null && stats.loaded.sets === 0 && this.source.finished);
    this.aiGroup.disabled = noData;
    this.noData.hidden = !noData;
    this.el.classList.toggle('ai-disabled', noData);
    if (noData) {
      if (this.lastStatsJson !== 'none') this.summary.replaceChildren(h('p.ai-empty', {}, 'No AI-credited commits to show.'));
      this.lastStatsJson = 'none';
      return;
    }
    if (json === this.lastStatsJson || !stats) return;
    this.lastStatsJson = json;
    this.renderStats(stats);
  }

  private renderStats(stats: AiStats): void {
    const { loaded, played } = stats;
    if (loaded.sets === 0) {
      this.summary.replaceChildren(h('p.ai-empty', {}, this.source.finished ? 'No AI-credited commits.' : 'No AI-credited commits loaded yet.'));
      return;
    }
    const totals = (t: { commits: number; assisted: number; agent: number }) =>
      `${formatCount(t.commits)} commits · ${formatCount(t.assisted)} AI-assisted · ${formatCount(t.agent)} by an agent`;
    const playedById = new Map(played.agents.map((a) => [a.id, a]));
    const rows = loaded.agents.map((a) => agentRow(a, playedById.get(a.id)));
    if (loaded.more_agents > 0) {
      rows.push(h('tr.ai-more', {}, h('td', { colspan: 3 }, `${formatCount(loaded.more_agents)} more agents`)));
    }
    this.summary.replaceChildren(
      h('p.ai-totals', { 'data-stat': 'loaded' }, h('span.ai-totals-label', {}, 'Loaded'), totals(loaded)),
      h('p.ai-totals', { 'data-stat': 'played' }, h('span.ai-totals-label', {}, 'Played'), totals(played)),
      h(
        'table.ai-agents',
        {},
        h('caption.ai-vh', {}, 'Commits credited to each agent'),
        h('thead', {}, h('tr', {}, h('th', { scope: 'col' }, 'Agent'), h('th', { scope: 'col' }, 'Played'), h('th', { scope: 'col' }, 'Loaded'))),
        h('tbody', {}, ...rows),
      ),
    );
  }
}

function switchRow(id: string, label: string, input: HTMLInputElement, control: string): HTMLElement {
  return h(
    'div.control.control-bool',
    { 'data-control': control },
    h('div.control-row', {}, h('label.control-label', { for: id }, label), h('span.switch', {}, input, h('span.switch-track', { 'aria-hidden': 'true' }))),
  );
}

function agentRow(agent: AiAgentStats, played: AiAgentStats | undefined): HTMLElement {
  const swatch = h('span.ai-swatch', { 'aria-hidden': 'true' });
  swatch.style.backgroundColor = `#${agent.colour}`;
  const models = agent.models.map((m) => `${m.model} (${formatCount(m.commits)})`);
  if (agent.other_models > 0) models.push(`other models (${formatCount(agent.other_models)})`);
  return h(
    'tr',
    { 'data-agent': agent.id },
    h(
      'th',
      { scope: 'row' },
      h('span.ai-agent', {}, swatch, h('span.ai-agent-name', {}, agent.name)),
      models.length > 0 ? h('span.ai-models', {}, models.join(', ')) : null,
      agent.as_author > 0 ? h('span.ai-models', {}, `as author: ${formatCount(agent.as_author)}`) : null,
    ),
    h('td', { 'data-stat': 'played' }, formatCount(played?.commits ?? 0)),
    h('td', { 'data-stat': 'loaded' }, formatCount(agent.commits)),
  );
}

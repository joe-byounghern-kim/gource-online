// SPDX-License-Identifier: GPL-3.0-or-later

// Collapsible control panel: four sections of Gource options. Live sections
// call set_option as controls change; the Filters section re-simulates on
// Apply (set_option for each filter, then restart()).

import { h, icon } from '../dom';
import {
  HIDE_ELEMENTS,
  OPTION_DEFS,
  SECTIONS,
  normaliseHide,
  normaliseValue,
  type OptionDef,
  type RangeDef,
  type SectionId,
} from '@gource-online/viewer/options';

export interface PanelHost {
  /** Applies one option; returns an error message or null. */
  setOption(name: string, value: string): string | null;
  /** Applies the filter options, then restarts the simulation; returns the first rejected option. */
  applyFilters(values: Array<[string, string]>): { name: string; message: string } | null;
  /** A section built by its feature module (F1: 'style'); the generic controls are skipped. */
  section?(id: SectionId): HTMLElement | null;
}

interface Control {
  def: OptionDef;
  set(value: string): void;
  error: HTMLElement;
  root: HTMLElement;
}

const HIDE_LABELS: Record<string, string> = {
  date: 'Date',
  users: 'Users',
  tree: 'Tree',
  files: 'Files',
  usernames: 'User names',
  filenames: 'File names',
  dirnames: 'Dir names',
  progress: 'Progress',
  bloom: 'Bloom',
  mouse: 'Mouse',
  root: 'Root',
  decorations: 'Decorations',
  'pull-requests': 'Pull requests',
};

function toSlider(def: RangeDef, value: number): number {
  if (!def.log) return value;
  return Math.log(value / def.min) / Math.log(def.max / def.min);
}

function fromSlider(def: RangeDef, pos: number): number {
  if (!def.log) return pos;
  const v = def.min * Math.pow(def.max / def.min, pos);
  const digits = v < 1 ? 2 : v < 10 ? 1 : 0;
  return Number(v.toFixed(digits));
}

function formatRange(def: RangeDef, value: number): string {
  const decimals = def.step >= 1 ? 0 : def.step >= 0.1 && !def.log ? 1 : 2;
  const text = Number.isInteger(value) && def.step >= 1 ? String(value) : value.toFixed(decimals).replace(/\.?0+$/, '') || '0';
  return def.unit ? `${text}${def.unit === '×' ? '×' : ` ${def.unit}`}` : text;
}

export class ControlPanel {
  readonly el: HTMLElement;
  private readonly host: PanelHost;
  private readonly controls = new Map<string, Control>();
  private readonly pendingFilters = new Map<string, string>();
  private readonly applyBtn: HTMLButtonElement;
  private readonly applyStatus: HTMLElement;
  private values: Map<string, string>;
  private debounce = new Map<string, number>();
  private frame = new Map<string, string>();
  private raf = 0;

  constructor(host: PanelHost, values: Map<string, string>, onClose: () => void) {
    this.host = host;
    this.values = values;
    this.applyBtn = h('button.btn.btn-primary.apply-filters', { type: 'button', disabled: true, onclick: () => this.apply() }, 'Apply filters');
    this.applyStatus = h('span.apply-status', { 'aria-live': 'polite' });

    const body = h('div.panel-body');
    for (const section of SECTIONS) {
      const custom = host.section?.(section.id);
      if (custom) {
        body.append(custom);
        continue;
      }
      const defs = OPTION_DEFS.filter((d) => d.section === section.id);
      const content = h('div.section-content');
      if (section.note) content.append(h('p.section-note', {}, section.note));
      for (const def of defs) content.append(this.build(def, section.id));
      if (section.id === 'filters') content.append(h('div.filter-actions', {}, this.applyBtn, this.applyStatus));
      body.append(
        h(
          'details.panel-section',
          { 'data-section': section.id, open: section.id === 'look' || section.id === 'playback' },
          h('summary', {}, section.title),
          content,
        ),
      );
    }

    this.el = h(
      'aside.panel',
      { id: 'control-panel', 'aria-label': 'Visualisation settings' },
      h(
        'header.panel-head',
        {},
        h('h2', {}, 'Settings'),
        h('button.icon-btn', { type: 'button', 'aria-label': 'Close settings', onclick: onClose }, icon('close')),
      ),
      body,
    );
  }

  /** Reflects a value changed elsewhere (playback bar, URL, engine key press). */
  setValue(name: string, value: string): void {
    this.values.set(name, value);
    if (name === 'hide') {
      this.controls.get('hide')?.set(value);
      return;
    }
    this.controls.get(name)?.set(value);
  }

  private current(def: OptionDef): string {
    return this.values.get(def.name) ?? def.default;
  }

  private build(def: OptionDef, section: SectionId): HTMLElement {
    if (def.name === 'hide') return this.buildHide(def);
    const id = `opt-${def.name}`;
    const error = h('span.control-error', { id: `${id}-error`, 'aria-live': 'polite' });
    const label = h('label.control-label', { for: id }, def.label);
    const live = section !== 'filters';
    const commit = (value: string, mode: 'now' | 'frame' | 'debounce' = 'now') => {
      if (!live) {
        this.pendingFilters.set(def.name, value);
        this.applyBtn.disabled = false;
        this.applyStatus.textContent = 'Not applied yet';
        return;
      }
      if (mode === 'frame') this.queueFrame(def.name, value);
      else if (mode === 'debounce') {
        window.clearTimeout(this.debounce.get(def.name));
        this.debounce.set(def.name, window.setTimeout(() => this.send(def.name, value), 300));
      } else this.send(def.name, value);
    };

    let input: HTMLElement;
    let set: (value: string) => void;
    let extra: HTMLElement | null = null;
    switch (def.kind) {
      case 'colour': {
        const picker = h('input.colour-input', { id, type: 'color', 'data-option': def.name });
        const hex = h('span.colour-hex');
        set = (v) => {
          const c = normaliseValue(def.name, v);
          picker.value = `#${c.toLowerCase()}`;
          hex.textContent = `#${c}`;
        };
        picker.addEventListener('input', () => {
          const v = picker.value.slice(1).toUpperCase();
          hex.textContent = `#${v}`;
          commit(v, 'frame');
        });
        input = h('div.colour-control', {}, picker, hex);
        break;
      }
      case 'range': {
        const slider = h('input.range-input', {
          id,
          type: 'range',
          min: def.log ? 0 : def.min,
          max: def.log ? 1 : def.max,
          step: def.log ? 0.001 : def.step,
          'data-option': def.name,
        });
        const out = h('output.range-value', { for: id });
        set = (v) => {
          const n = Number(v);
          slider.value = String(toSlider(def, Number.isFinite(n) ? n : Number(def.default)));
          out.textContent = formatRange(def, n);
        };
        slider.addEventListener('input', () => {
          const n = fromSlider(def, Number(slider.value));
          out.textContent = formatRange(def, n);
          commit(String(n), 'frame');
        });
        input = h('div.range-control', {}, slider, out);
        break;
      }
      case 'bool': {
        const box = h('input.switch-input', { id, type: 'checkbox', role: 'switch', 'data-option': def.name });
        set = (v) => {
          box.checked = (v === 'true') !== Boolean(def.invert);
        };
        box.addEventListener('change', () => commit(String(box.checked !== Boolean(def.invert))));
        input = h('span.switch', {}, box, h('span.switch-track', { 'aria-hidden': 'true' }));
        break;
      }
      case 'select': {
        const select = h('select.select-input', { id, 'data-option': def.name }, ...def.choices.map((c) => h('option', { value: c.value }, c.label)));
        set = (v) => {
          select.value = v;
        };
        select.addEventListener('change', () => commit(select.value));
        input = select;
        break;
      }
      case 'text': {
        const text = h('input.text-input', {
          id,
          type: 'text',
          spellcheck: 'false',
          autocomplete: 'off',
          placeholder: def.placeholder ?? '',
          'data-option': def.name,
        });
        set = (v) => {
          text.value = v;
        };
        text.addEventListener('input', () => commit(text.value.trim(), 'debounce'));
        text.addEventListener('keydown', (e) => {
          if (e.key === 'Enter' && !live) this.apply();
        });
        input = text;
        if (def.name === 'hash-seed') {
          extra = h(
            'button.btn.btn-small',
            {
              type: 'button',
              'data-action': 'reseed',
              onclick: () => {
                const seed = String(1 + Math.floor(Math.random() * 99999));
                text.value = seed;
                commit(seed);
              },
            },
            icon('dice'),
            'Reseed',
          );
        }
        break;
      }
      case 'list': {
        const area = h('textarea.list-input', {
          id,
          rows: 2,
          spellcheck: 'false',
          placeholder: def.placeholder ?? '',
          'data-option': def.name,
        });
        set = (v) => {
          area.value = v;
        };
        area.addEventListener('input', () => commit(normaliseValue(def.name, area.value), 'debounce'));
        input = area;
        break;
      }
    }

    const root = h(
      `div.control.control-${def.kind}`,
      { 'data-control': def.name },
      h('div.control-row', {}, label, input, extra),
      def.hint ? h('p.control-hint', {}, def.hint) : null,
      error,
    );
    root.querySelector('[data-option]')?.setAttribute('aria-describedby', `${id}-error`);
    const control: Control = { def, set, error, root };
    this.controls.set(def.name, control);
    set(this.current(def));
    return root;
  }

  private buildHide(def: OptionDef): HTMLElement {
    const error = h('span.control-error', { 'aria-live': 'polite' });
    const boxes = new Map<string, HTMLInputElement>();
    const grid = h('div.hide-grid', { role: 'group', 'aria-label': 'Hide elements' });
    const read = () => HIDE_ELEMENTS.filter((e) => boxes.get(e)!.checked).join(',');
    for (const element of HIDE_ELEMENTS) {
      const box = h('input', { type: 'checkbox', 'data-hide': element });
      box.addEventListener('change', () => this.send('hide', read()));
      boxes.set(element, box);
      grid.append(h('label.toggle-chip', {}, box, h('span', {}, HIDE_LABELS[element] ?? element)));
    }
    const set = (v: string) => {
      const on = new Set<string>(normaliseHide(v.split(',')));
      for (const [e, box] of boxes) box.checked = on.has(e);
    };
    const root = h('div.control.control-hide', { 'data-control': 'hide' }, h('span.control-label', {}, 'Hide'), grid, error);
    this.controls.set('hide', { def, set, error, root });
    set(this.current(def));
    return root;
  }

  private queueFrame(name: string, value: string): void {
    this.frame.set(name, value);
    if (this.raf) return;
    this.raf = requestAnimationFrame(() => {
      this.raf = 0;
      const batch = [...this.frame];
      this.frame.clear();
      for (const [n, v] of batch) this.send(n, v);
    });
  }

  private send(name: string, value: string): void {
    const err = this.host.setOption(name, value);
    const control = this.controls.get(name);
    if (control) showError(control, err);
    if (err === null) this.values.set(name, value);
  }

  private apply(): void {
    const values: Array<[string, string]> = [];
    for (const def of OPTION_DEFS) {
      if (def.section !== 'filters') continue;
      values.push([def.name, this.pendingFilters.get(def.name) ?? this.current(def)]);
    }
    for (const c of this.controls.values()) {
      if (c.def.section === 'filters') showError(c, null);
    }
    const err = this.host.applyFilters(values);
    if (err) {
      this.applyStatus.textContent = 'Not applied';
      const control = this.controls.get(err.name);
      if (control) showError(control, err.message);
      return;
    }
    for (const [n, v] of values) this.values.set(n, v);
    this.pendingFilters.clear();
    this.applyBtn.disabled = true;
    this.applyStatus.textContent = 'Applied: simulation restarted';
  }

  dispose(): void {
    if (this.raf) cancelAnimationFrame(this.raf);
    for (const t of this.debounce.values()) window.clearTimeout(t);
  }
}

/** Shows (or clears) a control's error, for the eye and for assistive technology. */
function showError(control: Control, message: string | null): void {
  control.error.textContent = message ?? '';
  control.root.classList.toggle('is-invalid', message !== null);
  const field = control.root.querySelector('[data-option]');
  if (message === null) field?.removeAttribute('aria-invalid');
  else field?.setAttribute('aria-invalid', 'true');
}

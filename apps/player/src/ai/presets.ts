// SPDX-License-Identifier: GPL-3.0-or-later

// Rule presets of the "My rules" box (F1 section 4.4). They only fill the
// textarea; the rules go through check_style and set_option like typed ones.
// The grammar is M2's: selectors human, agent, assisted, ai, ai:AGENT;
// properties cape, shades, hat, halo, glow, badge, none.

export interface RulePreset {
  id: string;
  label: string;
  rules: string;
}

export const RULE_PRESETS: readonly RulePreset[] = [
  { id: 'hero', label: 'Hero (built-in look)', rules: '' },
  { id: 'minimal', label: 'Minimal (badges only)', rules: 'ai => cape=off; shades=off; halo=off; glow=off' },
  { id: 'glowing-hats', label: 'Glowing hats', rules: 'assisted => cape=off; shades=off; hat=agent; glow=agent' },
];

/** Short help shown under the rules box. */
export const RULES_HELP =
  'One rule per line: selector => properties. Selectors: human, agent, assisted, ai, ai:AGENT (join with &, negate with !). ' +
  'Properties: cape, shades, hat, halo, glow, badge, none; values on, off, agent or RRGGBB. Lines starting with # are comments.';

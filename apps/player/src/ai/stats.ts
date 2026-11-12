// SPDX-License-Identifier: GPL-3.0-or-later

// `GourceApp.ai_stats()` (F1 AI-63): AI attribution numbers of the loaded log
// and of the commits played so far. The engine writes the JSON by hand; this
// reads it defensively so a malformed answer shows nothing instead of
// throwing.

export interface AiModelStats {
  model: string;
  commits: number;
}

export interface AiAgentStats {
  id: string;
  name: string;
  /** RRGGBB, validated. */
  colour: string;
  commits: number;
  as_author: number;
  models: AiModelStats[];
  /** Commits credited to models beyond the listed ones. */
  other_models: number;
}

export interface AiTotals {
  commits: number;
  assisted: number;
  agent: number;
  bot: number;
  agents: AiAgentStats[];
  more_agents: number;
}

export interface AiStats {
  registry: number;
  /** `sets`: distinct attribution sets in the log; 0 means no attribution data. */
  loaded: AiTotals & { sets: number };
  played: AiTotals;
}

const num = (v: unknown): number => (typeof v === 'number' && Number.isFinite(v) && v >= 0 ? v : 0);
const str = (v: unknown): string => (typeof v === 'string' ? v : '');

function readAgent(raw: unknown): AiAgentStats | null {
  if (!raw || typeof raw !== 'object') return null;
  const o = raw as Record<string, unknown>;
  const id = str(o.id);
  if (!id) return null;
  const colour = str(o.colour).toUpperCase();
  const models = Array.isArray(o.models)
    ? (o.models as unknown[]).flatMap((m) => {
        if (!m || typeof m !== 'object') return [];
        const r = m as Record<string, unknown>;
        const model = str(r.model);
        return model ? [{ model, commits: num(r.commits) }] : [];
      })
    : [];
  return {
    id,
    name: str(o.name) || id,
    colour: /^[0-9A-F]{6}$/.test(colour) ? colour : 'FFFFFF',
    commits: num(o.commits),
    as_author: num(o.as_author),
    models,
    other_models: num(o.other_models),
  };
}

function readTotals(raw: unknown): AiTotals {
  const o = raw && typeof raw === 'object' ? (raw as Record<string, unknown>) : {};
  return {
    commits: num(o.commits),
    assisted: num(o.assisted),
    agent: num(o.agent),
    bot: num(o.bot),
    agents: Array.isArray(o.agents) ? (o.agents as unknown[]).map(readAgent).filter((a): a is AiAgentStats => a !== null) : [],
    more_agents: num(o.more_agents),
  };
}

export function parseAiStats(json: string): AiStats | null {
  let raw: unknown;
  try {
    raw = JSON.parse(json);
  } catch {
    return null;
  }
  if (!raw || typeof raw !== 'object') return null;
  const o = raw as Record<string, unknown>;
  const loadedRaw = o.loaded && typeof o.loaded === 'object' ? (o.loaded as Record<string, unknown>) : {};
  return {
    registry: num(o.registry),
    loaded: { ...readTotals(o.loaded), sets: num(loadedRaw.sets) },
    played: readTotals(o.played),
  };
}

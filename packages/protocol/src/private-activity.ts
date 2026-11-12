// SPDX-License-Identifier: MIT
import type { ActivityRange, PaidActivityFeature } from './activity.js';
import { parseRepoUrl, validateBranch } from './repo-url.js';

/** Result metadata only, never authorization or permission to publish. */
export interface ActivityResultProvenance {
  visibility: 'public' | 'private' | 'internal' | 'mixed';
  publication: 'review-required' | 'forbidden';
}

/** Display-only source access state, not account or repository authority. */
export type ActivitySourceAccessState = 'available' | 'approval_required'
  | 'reauthentication_required' | 'access_lost' | 'unavailable';

/** Copy only valid provenance fields. Public results still require review. */
export function readActivityResultProvenance(value: unknown): Readonly<ActivityResultProvenance> | null {
  try {
    if (value === null || typeof value !== 'object' || Array.isArray(value)) return null;
    const { visibility, publication } = value as Record<string, unknown>;
    if (visibility !== 'public' && visibility !== 'private' && visibility !== 'internal' && visibility !== 'mixed') return null;
    const expectedPublication = visibility === 'public' ? 'review-required' : 'forbidden';
    if (publication !== expectedPublication) return null;
    return Object.freeze({ visibility, publication: expectedPublication });
  } catch { return null; }
}

/** Accept only exact primitive states, without coercing provider data. */
export function readActivitySourceAccessState(value: unknown): ActivitySourceAccessState | null {
  switch (value) {
    case 'available':
    case 'approval_required':
    case 'reauthentication_required':
    case 'access_lost':
    case 'unavailable':
      return value;
    default:
      return null;
  }
}

// ---- Private repositories: install, discovery and access ----

/** POST /api/activity/private/install. The purpose decides which read permissions the installation must grant. */
export interface PrivateInstallRequest { purpose: PaidActivityFeature }
/** GitHub's install page for the service's App. The state is opaque, single-use and grants nothing. */
export interface PrivateInstallResponse { install_url: string }
/** The install callback redirects home with `?private-install=<outcome>`. It never names an installation or a reason:
 * `pending` means an organization owner still has to approve, `unavailable` covers everything else. */
export type PrivateInstallOutcome = 'ready' | 'pending' | 'unavailable';
export const PRIVATE_INSTALL_PARAM = 'private-install';

/** A private or internal repository the signed-in user may read through an installation of the service's App.
 * Only usable repositories are listed: nothing about the ones the user cannot read. */
export interface PrivateActivityRepository {
  /** Opaque discovery-local receipt, not a repository, installation or account id. */
  id: string;
  url: string;
  name: string;
  default_branch: string;
  head: string;
  fork: boolean;
  visibility: 'private' | 'internal';
  /** Whether the repository belongs to the user or to an organization. A display hint only. */
  owner: 'personal' | 'organization';
}
/** POST /api/activity/private/discover. Private discovery is a request of its own, apart from public discovery, and counts
 * as its own search, unless it is chained: `public_discovery` names the public discovery of the same Find, over the same
 * range (another range is refused), and the first private discovery chained to it is not counted again, even if it fails. A page runs it only when the person asks to
 * find projects, and not when GitHub has just reported no usable installation. Its repositories can be analyzed together
 * with public ones: see {@link PrivateConsentRequest}. */
export interface HostedPrivateDiscoveryRequest { range: ActivityRange; public_discovery?: string }
export interface HostedPrivateDiscoveryResponse {
  discovery: string;
  repositories: readonly PrivateActivityRepository[];
  coverage: 'incomplete';
  expires_at: string;
}

/** The most repositories one private consent may select. */
export const PRIVATE_SELECTION_LIMIT = 10;
/** The most repositories, private and public together, one combined consent may select. */
export const COMBINED_SELECTION_LIMIT = 50;
/** POST /api/activity/private/consent. Each selected repository is checked at GitHub again before the consent is issued.
 * Verified-email matching is not offered for private sources: omit it or send false.
 *
 * A combined selection also names public repositories from one public discovery over the same range, with
 * `public_discovery` and `public_repository_ids` (both or neither). It is for `user-activity` only, needs at least one
 * private repository and takes at most {@link COMBINED_SELECTION_LIMIT} repositories in all. Its result is one owner-only
 * analysis (`visibility: 'mixed'`), never publishable. A selection of public repositories only uses the public routes. */
export interface PrivateConsentRequest {
  discovery: string;
  /** Receipts from the private discovery, at most {@link PRIVATE_SELECTION_LIMIT}. */
  repository_ids: string[];
  purpose: PaidActivityFeature;
  link_self: true;
  verified_email_match?: false;
  /** The public discovery of a combined selection. */
  public_discovery?: string;
  /** Receipts from that public discovery. */
  public_repository_ids?: string[];
  /** Personal activity only: also show the other contributors of the selected repositories, under anonymous actors that
   * name no one (cloud #96). Recorded in the consent; the analysis request must repeat it. */
  other_contributors?: true;
}
export interface PrivateConsentResponse { consent: string; expires_at: string }
/** POST /api/activity/private/analyze: personal activity across the consented private repositories, and the public ones
 * of a combined selection. The consent must be for `user-activity`. The discoveries, the repositories and the range must
 * be the consented ones. Answers a job (202). */
export interface PrivateAnalyzeRequest {
  discovery: string;
  consent: string;
  repository_ids: string[];
  range: ActivityRange;
  idempotency_key: string;
  continue_with_available?: boolean;
  public_discovery?: string;
  public_repository_ids?: string[];
  /** Must match the consent. */
  other_contributors?: true;
}
/** POST /api/activity/private/repository: pull request activity for exactly one consented private repository.
 * The consent must be for `pull-request-reviews` and name only this repository. `since` and `until` must be the
 * discovery's range. Answers a job (202). */
export interface PrivateRepositoryRequest {
  url: string;
  consent: string;
  since: string;
  until: string;
  pull_request_mode: 'full';
  idempotency_key: string;
  continue_with_available?: boolean;
}

const record = (value: unknown): Record<string, unknown> | null =>
  value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null;
const receipt = (value: unknown): value is string => typeof value === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(value);
const instant = (value: unknown): value is string => {
  if (typeof value !== 'string' || !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{3})?Z$/.test(value)) return false;
  const time = Date.parse(value);
  return Number.isFinite(time) && new Date(time).toISOString() === (value.length === 20 ? value.replace('Z', '.000Z') : value);
};

/** GET /api/activity/private/access: how many of the signed-in user's own installations of the App GitHub confirms
 *  are usable now, at most 10. A count only, never an installation, account or repository. */
export interface PrivateAccessStatus { installations: number }
export const MAX_PRIVATE_ACCESS_INSTALLATIONS = 10;
export function readPrivateAccessStatus(value: unknown): Readonly<PrivateAccessStatus> | null {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return null;
  const o = value as Record<string, unknown>;
  if (Object.keys(o).length !== 1 || !Number.isSafeInteger(o.installations) || (o.installations as number) < 0
    || (o.installations as number) > MAX_PRIVATE_ACCESS_INSTALLATIONS) return null;
  return Object.freeze({ installations: o.installations as number });
}

export function readPrivateInstallOutcome(value: unknown): PrivateInstallOutcome | null {
  return value === 'ready' || value === 'pending' || value === 'unavailable' ? value : null;
}

/** Only GitHub's own install page for one App, with exactly one state parameter. Anything else is not followed. */
export function readPrivateInstallResponse(value: unknown): PrivateInstallResponse | null {
  try {
    const o = record(value);
    if (!o || Object.keys(o).length !== 1 || typeof o.install_url !== 'string' || o.install_url.length > 2048) return null;
    const url = new URL(o.install_url);
    // Only the canonical spelling: no backslashes, dot segments, case or whitespace the URL parser would quietly fix.
    if (url.href !== o.install_url || url.origin !== 'https://github.com' || url.username || url.password || url.hash
      || !/^\/apps\/[a-z0-9](?:[a-z0-9-]{0,98}[a-z0-9])?\/installations\/new$/.test(url.pathname)) return null;
    const keys = [...url.searchParams.keys()];
    if (keys.length !== 1 || keys[0] !== 'state' || !/^[A-Za-z0-9_.-]{1,1024}$/.test(url.searchParams.get('state')!)) return null;
    return Object.freeze({ install_url: o.install_url });
  } catch { return null; }
}

/** Copy only the private discovery contract. No installation, account or provider fields propagate. */
export function readActivityPrivateDiscovery(value: unknown): HostedPrivateDiscoveryResponse | null {
  try {
    const o = record(value);
    if (!o || !receipt(o.discovery) || !instant(o.expires_at) || o.coverage !== 'incomplete' ||
      !Array.isArray(o.repositories) || o.repositories.length > 50) return null;
    const repositories: PrivateActivityRepository[] = [], seen = new Set<string>();
    for (const raw of o.repositories) {
      const r = record(raw);
      if (!r || !receipt(r.id) || seen.has(r.id) || typeof r.url !== 'string' || r.url.length > 512 ||
        typeof r.name !== 'string' || typeof r.default_branch !== 'string' || !r.default_branch ||
        r.default_branch.length > 256 || validateBranch(r.default_branch) || typeof r.head !== 'string' ||
        !/^(?:[a-f0-9]{40}|[a-f0-9]{64})$/.test(r.head) || typeof r.fork !== 'boolean' ||
        (r.visibility !== 'private' && r.visibility !== 'internal') || (r.owner !== 'personal' && r.owner !== 'organization')) return null;
      const parsed = parseRepoUrl(`${r.url}.git`);
      if (!parsed.ok || parsed.repo.host !== 'github.com' || parsed.repo.path !== r.name || r.url !== `https://github.com/${r.name}`) return null;
      repositories.push(Object.freeze({ id: r.id, url: r.url, name: r.name, default_branch: r.default_branch, head: r.head,
        fork: r.fork, visibility: r.visibility, owner: r.owner }));
      seen.add(r.id);
    }
    return Object.freeze({ discovery: o.discovery, repositories: Object.freeze(repositories), coverage: 'incomplete', expires_at: o.expires_at });
  } catch { return null; }
}

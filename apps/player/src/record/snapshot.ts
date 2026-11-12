// SPDX-License-Identifier: GPL-3.0-or-later

/** The bounded text contract of immutable activity snapshot settings. */
export function validSnapshotSetting(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= 1024 && value.trim() === value &&
    !value.includes('@') && !value.includes('://') &&
    !/[|\u0000-\u001f\u007f\u2028\u2029]/.test(value) &&
    !/(?:gh[pousr]_|github_pat_)[A-Za-z0-9_]+/.test(value);
}

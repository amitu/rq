/**
 * Serializable types for the script package installation pipeline.
 * All types here cross RPC boundaries (ADR-034) — plain objects and discriminated unions only.
 *
 * @see ADR-082 D-5 (install orchestration sequence)
 * @see ADR-083 D-3 (install observability)
 */

// ---------------------------------------------------------------------------
// Install Request
// ---------------------------------------------------------------------------

export interface InstallRequest {
  readonly contextId: string;
  readonly packages: ReadonlyArray<InstallPackageSpec>;
}

export interface InstallPackageSpec {
  readonly packageName: string;
  readonly version: string | undefined;
}

// ---------------------------------------------------------------------------
// Install Events (streamed via StreamReader<InstallEvent>)
// ---------------------------------------------------------------------------

export type InstallEvent =
  | InstallEventResolving
  | InstallEventDownloading
  | InstallEventInstalled
  | InstallEventFailed
  | InstallEventAlreadyInstalled;

export interface InstallEventResolving {
  readonly type: 'resolving';
  readonly packageName: string;
  readonly version: string | undefined;
}

export interface InstallEventDownloading {
  readonly type: 'downloading';
  readonly packageName: string;
  readonly version: string | undefined;
  readonly downloadedBytes: number;
  readonly totalBytes: number | null;
}

export interface InstallEventInstalled {
  readonly type: 'installed';
  readonly packageName: string;
  readonly version: string | undefined;
  readonly resolvedVersion: string;
}

export interface InstallEventFailed {
  readonly type: 'failed';
  readonly packageName: string;
  readonly version: string | undefined;
  readonly errorCode: InstallErrorCode;
  readonly message: string;
}

export interface InstallEventAlreadyInstalled {
  readonly type: 'already-installed';
  readonly packageName: string;
  readonly version: string | undefined;
}

/**
 * Discriminated error codes for install failures.
 * Covers ADR-080 security prerequisites and ADR-087 blacklist.
 */
export type InstallErrorCode =
  | 'BLACKLISTED'
  | 'NOT_FOUND'
  | 'VERSION_NOT_FOUND'
  | 'NETWORK_ERROR'
  | 'NATIVE_ADDON'
  | 'INTEGRITY_FAILED'
  | 'INSTALL_FAILED'
  | 'TIMEOUT';

// ---------------------------------------------------------------------------
// Installed Package Query
// ---------------------------------------------------------------------------

export interface InstalledPackage {
  readonly packageName: string;
  readonly version: string;
}

// ---------------------------------------------------------------------------
// Uninstall
// ---------------------------------------------------------------------------

export interface UninstallRequest {
  readonly contextId: string;
  readonly packageName: string;
}

export type UninstallErrorCode = 'NOT_FOUND' | 'PERMISSION_DENIED' | 'UNINSTALL_FAILED';

export type UninstallResult =
  | { readonly status: 'removed' }
  | { readonly status: 'not-installed' }
  | { readonly status: 'failed'; readonly errorCode: UninstallErrorCode; readonly message: string };

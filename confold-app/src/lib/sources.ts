// (Picker build, step 1 of the source-picker rework — see confold.phase-3-datasources.)
// Pure source-picker logic: parse a pasted URL into a type + field values, assemble the backend
// `SourceSpec`, derive recents-chip slugs, and decide which actions (compare/sync/migrate) the
// capabilities allow. No DOM / no Tauri — fully unit-testable; the picker component is the shell.
import type { SourceSpec, SourceTypeInfo, FieldSpec, Capabilities } from "./types";

/** Form field values keyed by `FieldSpec.key` (dotted for nested, e.g. `auth.password`). */
export type FieldValues = Record<string, string>;

/** Emoji icon for a source type id — used by the type buttons and recents chips. */
export function iconFor(type: string): string {
  switch (type) {
    case "fs":
      return "📁";
    case "sftp":
      return "🌐";
    case "s3":
      return "☁️"; // matches the S3Kind catalog icon (backend) so chips and the picker agree
    default:
      return "📦";
  }
}

/**
 * Parse a pasted URL into a source type + flat field values so the picker can autofill the form.
 * Supports `file://` (and bare absolute paths) and `sftp://` / `ssh://`. Returns `null` if unrecognized.
 */
export function parseSourceUrl(raw: string): { kind: string; fields: FieldValues } | null {
  const url = raw.trim();
  if (url === "") return null;

  // Bare local path (no scheme): absolute POSIX, `~`, or a Windows drive.
  if (/^(\/|~|[A-Za-z]:[\\/])/.test(url)) {
    return { kind: "fs", fields: { root: url } };
  }

  const scheme = url.match(/^([a-z][a-z0-9+.-]*):\/\//i)?.[1]?.toLowerCase();
  if (!scheme) return null;

  if (scheme === "file") {
    // file:///abs/path  or  file://host/path → keep the path part.
    const afterScheme = url.slice("file://".length);
    const slash = afterScheme.indexOf("/");
    const path = slash >= 0 ? afterScheme.slice(slash) : "/" + afterScheme;
    return { kind: "fs", fields: { root: decodeURIComponent(path) } };
  }

  if (scheme === "sftp" || scheme === "ssh") {
    const rest = url.slice((scheme + "://").length);
    // [user[:password]@]host[:port][/path]
    const atIdx = rest.lastIndexOf("@");
    const authority = atIdx >= 0 ? rest.slice(0, atIdx) : "";
    const hostPart = atIdx >= 0 ? rest.slice(atIdx + 1) : rest;

    let username = "";
    let password: string | null = null;
    if (authority) {
      const colon = authority.indexOf(":");
      if (colon >= 0) {
        username = decodeURIComponent(authority.slice(0, colon));
        password = decodeURIComponent(authority.slice(colon + 1));
      } else {
        username = decodeURIComponent(authority);
      }
    }

    const slash = hostPart.indexOf("/");
    const hostPort = slash >= 0 ? hostPart.slice(0, slash) : hostPart;
    const path = slash >= 0 ? hostPart.slice(slash) : "";

    let host = hostPort;
    let port = "";
    const pc = hostPort.lastIndexOf(":");
    if (pc >= 0) {
      host = hostPort.slice(0, pc);
      port = hostPort.slice(pc + 1);
    }

    const fields: FieldValues = { host, "auth.method": "password" };
    if (port) fields.port = port;
    if (username) fields.username = username;
    if (path) fields.root = decodeURIComponent(path);
    if (password !== null) fields["auth.password"] = password;
    return { kind: "sftp", fields };
  }

  if (scheme === "s3") {
    // s3://[access:secret@]host[:port]/bucket[/prefix] — host:port is an S3-compatible endpoint (served
    // over http, e.g. the local `s3-demo` / MinIO). For AWS or an https endpoint, use the form fields.
    const rest = url.slice("s3://".length);
    const atIdx = rest.lastIndexOf("@");
    const authority = atIdx >= 0 ? rest.slice(0, atIdx) : "";
    const hostPart = atIdx >= 0 ? rest.slice(atIdx + 1) : rest;

    let accessKey = "";
    let secretKey: string | null = null;
    if (authority) {
      const colon = authority.indexOf(":");
      if (colon >= 0) {
        accessKey = decodeURIComponent(authority.slice(0, colon));
        secretKey = decodeURIComponent(authority.slice(colon + 1));
      } else {
        accessKey = decodeURIComponent(authority);
      }
    }

    const slash = hostPart.indexOf("/");
    const hostPort = slash >= 0 ? hostPart.slice(0, slash) : hostPart;
    const pathPart = slash >= 0 ? hostPart.slice(slash + 1) : "";
    const firstSlash = pathPart.indexOf("/");
    const bucket = firstSlash >= 0 ? pathPart.slice(0, firstSlash) : pathPart;
    const prefix = firstSlash >= 0 ? pathPart.slice(firstSlash + 1) : "";

    const fields: FieldValues = {};
    if (hostPort) fields.endpoint = `http://${hostPort}`;
    if (bucket) fields.bucket = decodeURIComponent(bucket);
    if (prefix) fields.prefix = decodeURIComponent(prefix);
    if (accessKey) fields.access_key_id = accessKey;
    if (secretKey !== null) fields.secret_access_key = secretKey;
    return { kind: "s3", fields };
  }

  return null;
}

/**
 * Assemble the backend `SourceSpec` from a kind id + flat field values — a pure passthrough into the
 * generic `{ kind, fields }` wire shape. Field parsing, defaults, and validation are the backend's job
 * (each `SourceKind::build`), so this stays the same for every backend.
 */
export function buildSpec(kind: string, fields: FieldValues): SourceSpec {
  return { kind, fields: { ...fields } };
}

/** The default value declared for a field key, or `""`. */
function defaultFor(info: SourceTypeInfo, key: string): string {
  return info.fields.find((f) => f.key === key)?.default ?? "";
}

/** Fields to show for the current values, applying each field's `show_when` (e.g. auth fields by method). */
export function visibleFields(info: SourceTypeInfo, fields: FieldValues): FieldSpec[] {
  return info.fields.filter((f) => {
    if (!f.show_when) return true;
    const [key, val] = f.show_when.split("=");
    const cur = fields[key] ?? defaultFor(info, key);
    return cur === val;
  });
}

/** Required, currently-visible fields that are still empty — the picker disables "confirm" while non-empty. */
export function missingRequired(info: SourceTypeInfo, fields: FieldValues): string[] {
  return visibleFields(info, fields)
    .filter((f) => f.required)
    .filter((f) => (fields[f.key] ?? f.default ?? "") === "")
    .map((f) => f.key);
}

/**
 * True if two specs point at the same source (ignoring secret/credential fields) — blocks compare-with-self.
 * Identity = kind + every non-secret field (secret-ness is declared per field in the `source_types()`
 * catalog), so it generalises to any backend without per-type code.
 */
export function sourcesEqual(a: SourceSpec, b: SourceSpec, types: SourceTypeInfo[]): boolean {
  if (a.kind !== b.kind) return false;
  const info = types.find((t) => t.id === a.kind);
  const secret = new Set(info?.fields.filter((f) => f.secret).map((f) => f.key));
  const identity = (s: SourceSpec) =>
    Object.keys(s.fields)
      .filter((k) => !secret.has(k))
      .sort()
      .map((k) => `${k}=${s.fields[k]}`)
      .join("&");
  return identity(a) === identity(b);
}

/**
 * Drop the secret/credential fields (per the `source_types()` catalog) from a spec, so recents can be
 * persisted to localStorage WITHOUT ever writing a password / secret key to disk. Full specs (with the
 * secret) stay in memory for the session; only the on-disk copy is stripped. A stripped spec that's later
 * reused will be missing a required field → the picker re-opens pre-filled so the user re-enters the secret.
 */
export function stripSecrets(spec: SourceSpec, types: SourceTypeInfo[]): SourceSpec {
  const info = types.find((t) => t.id === spec.kind);
  const secret = new Set(info?.fields.filter((f) => f.secret).map((f) => f.key));
  if (secret.size === 0) return spec;
  const fields: FieldValues = {};
  for (const k of Object.keys(spec.fields)) if (!secret.has(k)) fields[k] = spec.fields[k];
  return { kind: spec.kind, fields };
}

/** A short identity for a configured source — drives the recents chips (icon + label). No secrets. */
export function slugOf(spec: SourceSpec): { icon: string; label: string } {
  const icon = iconFor(spec.kind);
  const f = spec.fields;
  if (spec.kind === "fs") return { icon, label: f.root || "(path)" };
  if (spec.kind === "sftp") {
    const at = f.username ? `${f.username}@` : "";
    const tail = f.root && f.root !== "/" ? `:${f.root}` : "";
    return { icon, label: `${at}${f.host ?? ""}${tail}` };
  }
  if (spec.kind === "s3") {
    // [endpoint-host/]bucket[/prefix] — parallels sftp's host:path. Endpoint host (sans scheme) when a
    // custom S3-compatible server is used; bare bucket/prefix for AWS. Never the keys (secret-free).
    const host = (f.endpoint ?? "").replace(/^https?:\/\//i, "").replace(/\/+$/, "");
    const path = [f.bucket, f.prefix].map((s) => (s ?? "").trim()).filter(Boolean).join("/");
    return { icon, label: host ? `${host}/${path}` : path || "(bucket)" };
  }
  // Generic fallback so a new backend renders a sensible chip without editing this.
  return { icon, label: f.root || f.bucket || f.host || spec.kind };
}

/**
 * Which actions are offered given both sides' capabilities (mirrors the capability model):
 * - compare needs LIST + (READ or FINGERPRINT) on both,
 * - migrate (push origin→dest) needs READ on origin + WRITE on dest,
 * - sync (bidirectional) needs READ+WRITE on both.
 */
export function actionsFor(
  origin: Capabilities,
  dest: Capabilities,
): { compare: boolean; sync: boolean; migrate: boolean } {
  const canCompare = (c: Capabilities) => c.list && (c.read || c.fingerprint);
  return {
    compare: canCompare(origin) && canCompare(dest),
    migrate: origin.read && dest.write,
    sync: origin.read && origin.write && dest.read && dest.write,
  };
}

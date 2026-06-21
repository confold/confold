import { describe, it, expect } from "vitest";
import {
  parseSourceUrl,
  buildSpec,
  visibleFields,
  missingRequired,
  slugOf,
  actionsFor,
  sourcesEqual,
  stripSecrets,
} from "./sources";
import type { SourceTypeInfo, Capabilities } from "./types";

// Mirrors the backend `source_types()` SFTP entry (the fields the picker renders).
const SFTP_INFO: SourceTypeInfo = {
  id: "sftp",
  name: "SFTP",
  icon: "🌐",
  capabilities: { list: true, read: true, fingerprint: false, write: true },
  fields: [
    { key: "host", label: "Host", kind: "text", required: true, secret: false, default: null, options: [], show_when: null },
    { key: "port", label: "Port", kind: "number", required: false, secret: false, default: "22", options: [], show_when: null },
    { key: "username", label: "Username", kind: "text", required: true, secret: false, default: null, options: [], show_when: null },
    { key: "auth.method", label: "Authentication", kind: "select", required: true, secret: false, default: "password", options: ["password", "private_key"], show_when: null },
    { key: "auth.password", label: "Password", kind: "password", required: true, secret: true, default: null, options: [], show_when: "auth.method=password" },
    { key: "auth.pem", label: "Private key (PEM)", kind: "textarea", required: true, secret: true, default: null, options: [], show_when: "auth.method=private_key" },
    { key: "auth.passphrase", label: "Key passphrase", kind: "password", required: false, secret: true, default: null, options: [], show_when: "auth.method=private_key" },
    { key: "root", label: "Base directory", kind: "path", required: false, secret: false, default: "/", options: [], show_when: null },
  ],
};

const FS_RW: Capabilities = { list: true, read: true, fingerprint: false, write: true };
const WRITE_ONLY: Capabilities = { list: false, read: false, fingerprint: false, write: true };
const READ_ONLY: Capabilities = { list: true, read: true, fingerprint: false, write: false };

describe("parseSourceUrl", () => {
  it("parses a bare absolute path as an fs source", () => {
    expect(parseSourceUrl("/Users/me/fotos")).toEqual({ kind: "fs", fields: { root: "/Users/me/fotos" } });
    expect(parseSourceUrl("~/docs")?.kind).toBe("fs");
    expect(parseSourceUrl("C:\\Users\\me")?.kind).toBe("fs");
  });

  it("parses file:// URLs to the path", () => {
    expect(parseSourceUrl("file:///var/data")).toEqual({ kind: "fs", fields: { root: "/var/data" } });
    expect(parseSourceUrl("file://localhost/var/data")).toEqual({ kind: "fs", fields: { root: "/var/data" } });
  });

  it("parses a full sftp URL with user, password, port and path", () => {
    const r = parseSourceUrl("sftp://alice:s3cret@host.example:2222/srv/data");
    expect(r).toEqual({
      kind: "sftp",
      fields: {
        host: "host.example",
        port: "2222",
        username: "alice",
        root: "/srv/data",
        "auth.method": "password",
        "auth.password": "s3cret",
      },
    });
  });

  it("parses a minimal sftp URL (host only) and accepts ssh://", () => {
    expect(parseSourceUrl("sftp://nas.local")).toEqual({
      kind: "sftp",
      fields: { host: "nas.local", "auth.method": "password" },
    });
    expect(parseSourceUrl("ssh://bob@box")?.fields).toMatchObject({ host: "box", username: "bob" });
  });

  it("parses an s3 URL (key:secret@host:port/bucket/prefix) into S3 fields", () => {
    expect(parseSourceUrl("s3://confold:confold-secret@127.0.0.1:4566/data/sub")).toEqual({
      kind: "s3",
      fields: {
        endpoint: "http://127.0.0.1:4566",
        bucket: "data",
        prefix: "sub",
        access_key_id: "confold",
        secret_access_key: "confold-secret",
      },
    });
  });

  it("returns null for empty or unrecognized input", () => {
    expect(parseSourceUrl("")).toBeNull();
    expect(parseSourceUrl("   ")).toBeNull();
    expect(parseSourceUrl("not a url")).toBeNull();
    expect(parseSourceUrl("https://example.com")).toBeNull();
  });
});

describe("buildSpec", () => {
  it("wraps a kind + flat fields into the generic spec (fs)", () => {
    expect(buildSpec("fs", { root: "/a/b" })).toEqual({ kind: "fs", fields: { root: "/a/b" } });
  });

  it("passes sftp fields through unchanged (parsing + defaults are the backend's job)", () => {
    const fields = { host: "h", username: "u", "auth.method": "password", "auth.password": "p" };
    expect(buildSpec("sftp", fields)).toEqual({ kind: "sftp", fields });
  });

  it("is generic — an unknown kind just wraps (no throw)", () => {
    expect(buildSpec("s3", { bucket: "b" })).toEqual({ kind: "s3", fields: { bucket: "b" } });
  });
});

describe("visibleFields / missingRequired", () => {
  it("shows password fields for password auth, key fields for key auth", () => {
    const pwKeys = visibleFields(SFTP_INFO, { "auth.method": "password" }).map((f) => f.key);
    expect(pwKeys).toContain("auth.password");
    expect(pwKeys).not.toContain("auth.pem");

    const keyKeys = visibleFields(SFTP_INFO, { "auth.method": "private_key" }).map((f) => f.key);
    expect(keyKeys).toContain("auth.pem");
    expect(keyKeys).not.toContain("auth.password");
  });

  it("uses the field default when the discriminator is unset (password)", () => {
    // auth.method has default "password" → password field is visible without setting it.
    expect(visibleFields(SFTP_INFO, {}).map((f) => f.key)).toContain("auth.password");
  });

  it("reports required visible fields that are empty (and not the hidden ones)", () => {
    const missing = missingRequired(SFTP_INFO, { "auth.method": "password" });
    expect(missing).toContain("host");
    expect(missing).toContain("username");
    expect(missing).toContain("auth.password");
    expect(missing).not.toContain("auth.pem"); // hidden under password auth
    expect(missing).not.toContain("port"); // optional
    expect(missing).not.toContain("root"); // optional

    expect(missingRequired(SFTP_INFO, { host: "h", username: "u", "auth.password": "p" })).toEqual([]);
  });
});

describe("stripSecrets", () => {
  it("drops secret fields (per the catalog) but keeps the rest — so recents never persist credentials", () => {
    const spec = {
      kind: "sftp",
      fields: { host: "h", username: "u", "auth.method": "password", "auth.password": "hunter2", root: "/data" },
    };
    expect(stripSecrets(spec, [SFTP_INFO])).toEqual({
      kind: "sftp",
      fields: { host: "h", username: "u", "auth.method": "password", root: "/data" },
    });
  });

  it("returns the spec unchanged when the kind has no secret fields", () => {
    const spec = { kind: "fs", fields: { root: "/a/b" } };
    expect(stripSecrets(spec, [SFTP_INFO])).toEqual(spec); // unknown/no-secret kind → untouched
  });
});

describe("slugOf", () => {
  it("labels an fs source by its path", () => {
    expect(slugOf({ kind: "fs", fields: { root: "/a/b" } })).toEqual({ icon: "📁", label: "/a/b" });
  });

  it("labels an sftp source as user@host[:root], no secrets", () => {
    expect(slugOf({ kind: "sftp", fields: { host: "h", username: "u", "auth.password": "x", root: "/data" } }))
      .toEqual({ icon: "🌐", label: "u@h:/data" });
    expect(slugOf({ kind: "sftp", fields: { host: "h", username: "u", root: "/" } }).label).toBe("u@h");
  });

  it("labels an s3 source as [endpoint-host/]bucket/prefix, no secrets", () => {
    expect(slugOf({ kind: "s3", fields: { bucket: "my-bucket", prefix: "data" } }))
      .toEqual({ icon: "☁️", label: "my-bucket/data" });
    expect(
      slugOf({
        kind: "s3",
        fields: { endpoint: "http://127.0.0.1:9000", bucket: "data", prefix: "bulk", secret_access_key: "x" },
      }).label,
    ).toBe("127.0.0.1:9000/data/bulk");
  });

  it("falls back generically for an unknown kind (renders without per-type code)", () => {
    expect(slugOf({ kind: "webdav", fields: { host: "h" } })).toEqual({ icon: "📦", label: "h" });
  });
});

describe("sourcesEqual", () => {
  it("matches identical fs paths and differs otherwise", () => {
    expect(sourcesEqual({ kind: "fs", fields: { root: "/a" } }, { kind: "fs", fields: { root: "/a" } }, [])).toBe(true);
    expect(sourcesEqual({ kind: "fs", fields: { root: "/a" } }, { kind: "fs", fields: { root: "/b" } }, [])).toBe(false);
  });

  it("matches sftp by non-secret fields (ignoring credentials), differs across kinds", () => {
    const a = { kind: "sftp", fields: { host: "h", port: "22", username: "u", "auth.password": "x", root: "/d" } };
    const b = { kind: "sftp", fields: { host: "h", port: "22", username: "u", "auth.password": "DIFFERENT", root: "/d" } };
    const c = { kind: "sftp", fields: { host: "h", port: "2222", username: "u", "auth.password": "x", root: "/d" } };
    const types = [SFTP_INFO];
    expect(sourcesEqual(a, b, types)).toBe(true); // same location, different password (secret → ignored)
    expect(sourcesEqual(a, c, types)).toBe(false); // different port
    expect(sourcesEqual({ kind: "fs", fields: { root: "/d" } }, a, types)).toBe(false); // different kind
  });
});

describe("actionsFor", () => {
  it("offers all three when both sides are read/write", () => {
    expect(actionsFor(FS_RW, FS_RW)).toEqual({ compare: true, sync: true, migrate: true });
  });

  it("a write-only destination (e.g. a secrets vault) is a migrate target only", () => {
    const a = actionsFor(FS_RW, WRITE_ONLY);
    expect(a.migrate).toBe(true);
    expect(a.compare).toBe(false); // can't read/list the sink
    expect(a.sync).toBe(false);
  });

  it("a read-only destination can be compared but not migrated into", () => {
    const a = actionsFor(FS_RW, READ_ONLY);
    expect(a.compare).toBe(true);
    expect(a.migrate).toBe(false);
    expect(a.sync).toBe(false);
  });
});

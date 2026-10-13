import { spawnSync } from "node:child_process";
import {
  readFileSync,
  readdirSync,
  writeFileSync,
  existsSync,
  mkdirSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import { resolve, join } from "node:path";
const root = fileURLToPath(new URL("../", import.meta.url));
const version = JSON.parse(
  readFileSync(join(root, "src-tauri/tauri.conf.json"), "utf8"),
).version;
const packageVersion = JSON.parse(
  readFileSync(join(root, "package.json"), "utf8"),
).version;
const rustVersion = readFileSync(
  join(root, "src-tauri/Cargo.toml"),
  "utf8",
).match(/^version = "([^"]+)"/m)?.[1];
const coreVersion = readFileSync(
  join(root, "src-tauri/core/Cargo.toml"),
  "utf8",
).match(/^version = "([^"]+)"/m)?.[1];
if (version !== packageVersion || version !== rustVersion || version !== coreVersion) {
  console.error(
    "Synchronize package.json, both Cargo.toml files, and tauri.conf.json versions before building.",
  );
  process.exit(1);
}
const args = process.argv.slice(2);
const signed = args.includes("--signed");
const check = args.includes("--check");
if (
  args.some((a) => !["--signed", "--development", "--check"].includes(a)) ||
  (signed && args.includes("--development"))
) {
  console.error(
    "Usage: node scripts/release-macos.mjs --development|--signed [--check]",
  );
  process.exit(2);
}
if (!signed && !args.includes("--development")) {
  console.error("Choose --development or --signed explicitly.");
  process.exit(2);
}
if (process.platform !== "darwin") {
  console.error("macOS is required.");
  process.exit(1);
}
function run(command, argv, env = process.env, capture = false) {
  const r = spawnSync(command, argv, {
    cwd: root,
    env,
    encoding: "utf8",
    stdio: capture ? "pipe" : "inherit",
  });
  if (r.error || r.status !== 0) {
    console.error(`Failed: ${command}`);
    process.exit(1);
  }
  return r.stdout || "";
}
const environment = { ...process.env, CI: "true" };
if (signed) {
  const identity = environment.APPLE_SIGNING_IDENTITY;
  if (!identity?.startsWith("Developer ID Application:")) {
    console.error(
      "APPLE_SIGNING_IDENTITY must select a Developer ID Application certificate.",
    );
    process.exit(1);
  }
  const identities = run(
    "/usr/bin/security",
    ["find-identity", "-v", "-p", "codesigning"],
    environment,
    true,
  );
  if (!identities.includes(`"${identity}"`)) {
    console.error(
      "The requested valid signing certificate is not in the keychain.",
    );
    process.exit(1);
  }
  const appleId =
    environment.APPLE_ID &&
    environment.APPLE_PASSWORD &&
    environment.APPLE_TEAM_ID;
  const api =
    environment.APPLE_API_ISSUER &&
    environment.APPLE_API_KEY &&
    environment.APPLE_API_KEY_PATH &&
    existsSync(environment.APPLE_API_KEY_PATH);
  if (!appleId && !api) {
    console.error(
      "Provide notarization credentials via environment variables. See docs/release.md.",
    );
    process.exit(1);
  }
} else {
  // Development output must never use a personal signing identity by accident.
  for (const key of Object.keys(environment))
    if (key.startsWith("APPLE_")) delete environment[key];
}
if (check) {
  console.log(`Preflight passed: ${signed ? "signed" : "development"} build.`);
  process.exit(0);
}
run("npm", ["test"], environment);
run(
  "cargo",
  ["test", "--manifest-path", "src-tauri/Cargo.toml", "--locked", "--workspace"],
  environment,
);
const config = signed
  ? []
  : [
      "--config",
      JSON.stringify({ bundle: { macOS: { signingIdentity: "-" } } }),
    ];
run(
  "npm",
  ["run", "tauri", "--", "build", "--bundles", "app,dmg", ...config],
  environment,
);
const bundle = join(root, "src-tauri/target/release/bundle");
const app = join(bundle, "macos/ClamAV Desktop.app");
run("/usr/bin/codesign", ["--verify", "--deep", "--strict", app], environment);
if (signed) {
  run("/usr/sbin/spctl", ["--assess", "--type", "execute", app], environment);
  run("/usr/bin/xcrun", ["stapler", "validate", app], environment);
}
const architecture = process.arch === "arm64" ? "aarch64" : "x64";
const artifacts = readdirSync(join(bundle, "dmg")).filter(
  (n) => n.endsWith(`_${architecture}.dmg`) && n.includes(`_${version}_`),
);
if (artifacts.length !== 1) {
  console.error(
    "Expected exactly one current-version DMG for this architecture.",
  );
  process.exit(1);
}
for (const file of artifacts)
  run("/usr/bin/hdiutil", ["verify", join(bundle, "dmg", file)], environment);
const output = join(bundle, "verification");
mkdirSync(output, { recursive: true });
const entries = artifacts.map((file) => ({
  file,
  sha256: createHash("sha256")
    .update(readFileSync(join(bundle, "dmg", file)))
    .digest("hex"),
}));
writeFileSync(
  join(output, "release.json"),
  JSON.stringify(
    {
      version,
      architecture: process.arch,
      development: !signed,
      appNotarizationVerified: signed,
      artifacts: entries,
    },
    null,
    2,
  ) + "\n",
);
writeFileSync(
  join(output, "SHA256SUMS"),
  entries.map((e) => `${e.sha256}  ${e.file}`).join("\n") + "\n",
);
console.log(
  `Verified ${signed ? "signed/notarized app" : "ad-hoc development app"} and DMG. Checksums: ${output}`,
);

import { chmod, copyFile, mkdir, rm, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const webUiRoot = resolve(scriptDir, "..");
const targetFromTauri = process.env.TAURI_ENV_TARGET_TRIPLE;
const target =
  targetFromTauri ??
  execFileSync("rustc", ["--print", "host-tuple"], { encoding: "utf8" }).trim();
const profile = process.argv.includes("--release") ? "release" : "debug";
const bundle = process.argv.includes("--bundle");
const appRoot = bundle
  ? join(
      webUiRoot,
      "src-tauri",
      "target",
      profile,
      "bundle",
      "macos",
      "mew.app",
    )
  : join(webUiRoot, "src-tauri", "target", "debug", "mew.app");
const helperSource = join(
  webUiRoot,
  "src-tauri",
  "binaries",
  `mew-cef-host-helper-${target}`,
);
const frameworksRoot = join(appRoot, "Contents", "Frameworks");
const helperVariants = [
  "Helper (GPU)",
  "Helper (Renderer)",
  "Helper (Plugin)",
  "Helper (Alerts)",
  "Helper",
];

if (process.platform !== "darwin") {
  console.log("skipping CEF helper app preparation on non-macOS");
  process.exit(0);
}

// CEF derives helper bundle names from CFBundleExecutable, not the outer
// product bundle directory (`mew.app`).
const appExecutable = "mew-desktop";
const version = "0.2.0";
const identifier = "ai.mew.mew";

for (const variant of helperVariants) {
  const executable = `${appExecutable} ${variant}`;
  const appPath = join(frameworksRoot, `${executable}.app`);
  const contentsPath = join(appPath, "Contents");
  const executablePath = join(contentsPath, "MacOS", executable);

  await rm(appPath, { recursive: true, force: true });
  await mkdir(join(contentsPath, "MacOS"), { recursive: true });
  await mkdir(join(contentsPath, "Resources"), { recursive: true });
  await mkdir(join(contentsPath, "Frameworks"), { recursive: true });
  await copyFile(helperSource, executablePath);
  await chmod(executablePath, 0o755);
  await writeFile(
    join(contentsPath, "Info.plist"),
    helperInfoPlist({ executable, version, identifier }),
  );
}

if (bundle) {
  const signingIdentity = process.env.APPLE_SIGNING_IDENTITY || "-";
  execFileSync(
    "codesign",
    ["--force", "--deep", "--sign", signingIdentity, appRoot],
    { stdio: "inherit" },
  );
}

console.log(
  `prepared ${helperVariants.length} CEF helper app bundles in ${frameworksRoot}`,
);

function helperInfoPlist({ executable, version, identifier }) {
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>mew</string>
  <key>CFBundleIdentifier</key>
  <string>${identifier}</string>
  <key>CFBundleDisplayName</key>
  <string>mew</string>
  <key>CFBundleExecutable</key>
  <string>${executable}</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleVersion</key>
  <string>${version}</string>
  <key>CFBundleShortVersionString</key>
  <string>${version}</string>
  <key>LSUIElement</key>
  <true/>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
  <key>LSEnvironment</key>
  <dict>
    <key>MallocNanoZone</key>
    <string>0</string>
  </dict>
</dict>
</plist>
`;
}

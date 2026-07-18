import { cp, mkdir, readdir, rm, symlink, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { homedir, platform, arch } from "node:os";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const webUiRoot = resolve(scriptDir, "..");
const packageDestination = join(
  webUiRoot,
  "src-tauri",
  "cef",
  "Chromium Embedded Framework.framework",
);
const runtimeDestination = join(
  webUiRoot,
  "src-tauri",
  "target",
  "Frameworks",
  "Chromium Embedded Framework.framework",
);
const developmentResourcesDestination = join(
  webUiRoot,
  "src-tauri",
  "target",
  "debug",
  "Resources",
);
const developmentLibrariesDestination = join(
  webUiRoot,
  "src-tauri",
  "target",
  "debug",
  "Libraries",
);
const developmentBundleDestination = join(
  webUiRoot,
  "src-tauri",
  "target",
  "debug",
  "mew.app",
);
const required = process.argv.includes("--required");
const link = process.argv.includes("--link");

if (platform() !== "darwin") {
  if (required) throw new Error("CEF bundling is currently macOS-only");
  console.log("skipping CEF preparation on non-macOS");
  process.exit(0);
}

const source = await findFramework();
if (!source) {
  const message =
    "CEF framework not found; set MEW_CEF_FRAMEWORK_SOURCE or CEF_PATH to a CEF distribution";
  if (required) throw new Error(message);
  console.warn(`skipping optional CEF preparation: ${message}`);
  process.exit(0);
}

await mkdir(dirname(packageDestination), { recursive: true });
await rm(packageDestination, { recursive: true, force: true });
if (link) {
  await mkdir(dirname(runtimeDestination), { recursive: true });
  await rm(runtimeDestination, { recursive: true, force: true });
  await symlink(source, packageDestination, "dir");
  await symlink(source, runtimeDestination, "dir");
  console.log(`linked CEF framework from ${source}`);
} else {
  await cp(source, packageDestination, { recursive: true });
  await mkdir(dirname(runtimeDestination), { recursive: true });
  await rm(runtimeDestination, { recursive: true, force: true });
  await cp(source, runtimeDestination, { recursive: true });
  await rm(developmentResourcesDestination, { recursive: true, force: true });
  await cp(join(source, "Resources"), developmentResourcesDestination, {
    recursive: true,
  });
  await rm(developmentLibrariesDestination, { recursive: true, force: true });
  await cp(join(source, "Libraries"), developmentLibrariesDestination, {
    recursive: true,
  });
  for (const library of ["libEGL.dylib", "libGLESv2.dylib"]) {
    await cp(
      join(source, "Libraries", library),
      join(dirname(developmentLibrariesDestination), library),
    );
  }
  console.log(`copied CEF framework from ${source}`);
}

await writeDevelopmentBundle();

// `tauri dev` runs an unbundled executable, but Chromium anchors its
// Mach-port rendezvous names to the main bundle identifier. Write a
// synthetic bundle next to the development executable so the CEF host can
// point CEF at it and the browser and helper processes agree on a name.
async function writeDevelopmentBundle() {
  const infoPlist = `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleName</key>
	<string>mew</string>
	<key>CFBundleIdentifier</key>
	<string>ai.mew.mew</string>
	<key>CFBundleDisplayName</key>
	<string>mew</string>
	<key>CFBundleExecutable</key>
	<string>mew</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleVersion</key>
	<string>0.2.0</string>
	<key>CFBundleShortVersionString</key>
	<string>0.2.0</string>
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
  const contentsDir = join(developmentBundleDestination, "Contents");
  await mkdir(contentsDir, { recursive: true });
  await writeFile(join(contentsDir, "Info.plist"), infoPlist);
  console.log(`wrote development bundle at ${developmentBundleDestination}`);
}

async function findFramework() {
  const frameworkName = "Chromium Embedded Framework.framework";
  const cefArch = arch() === "arm64" ? "aarch64" : "x86_64";
  const configured = process.env.MEW_CEF_FRAMEWORK_SOURCE;
  const roots = [
    configured,
    process.env.CEF_PATH,
    join(homedir(), ".local", "share", "cef"),
  ].filter(Boolean);

  for (const root of roots) {
    const direct = resolve(root, frameworkName);
    if (await isDirectory(direct)) return direct;

    const architectureRoot = resolve(root, `cef_macos_${cefArch}`);
    const architectureFramework = join(architectureRoot, frameworkName);
    if (await isDirectory(architectureFramework)) return architectureFramework;

    if (!(await isDirectory(root))) continue;
    for (const version of await readdir(root, { withFileTypes: true })) {
      if (!version.isDirectory()) continue;
      const candidate = join(root, version.name, `cef_macos_${cefArch}`, frameworkName);
      if (await isDirectory(candidate)) return candidate;
    }
  }
  return null;
}

async function isDirectory(path) {
  try {
    const entries = await readdir(path);
    return entries.length > 0;
  } catch {
    return false;
  }
}

// Проставляет версию во все манифесты: node scripts/set-version.mjs 1.2.3
import { readFileSync, writeFileSync } from "node:fs";

const version = process.argv[2]?.replace(/^v/, "");
if (!version || !/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
  console.error("Использование: node scripts/set-version.mjs <semver>");
  process.exit(1);
}

function editJson(path) {
  const json = JSON.parse(readFileSync(path, "utf8"));
  json.version = version;
  writeFileSync(path, JSON.stringify(json, null, 2) + "\n");
}

editJson("app/package.json");
editJson("app/src-tauri/tauri.conf.json");

const cargo = readFileSync("Cargo.toml", "utf8");
const updated = cargo.replace(
  /(\[workspace\.package\][^[]*?\nversion = )"[^"]*"/,
  `$1"${version}"`,
);
if (updated === cargo && !cargo.includes(`version = "${version}"`)) {
  console.error("Не нашёл version в [workspace.package]");
  process.exit(1);
}
writeFileSync("Cargo.toml", updated);

console.log(`Версия ${version}`);

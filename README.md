# ScamLauncher

Лаунчер Minecraft-сборок: сборки лежат на Яндекс Диске, сам лаунчер обновляется из GitHub Releases.
Полный план — в [PLAN.md](PLAN.md), публикация сборок — в [docs/PUBLISHING.md](docs/PUBLISHING.md).

## Установка

Скачай файл под свою ОС из [последнего релиза](https://github.com/semakol/ScamLauncher/releases/latest):

| ОС | Файл |
|---|---|
| Windows | `ScamLauncher_x.y.z_x64-setup.exe` |
| macOS (M1 и новее) | `ScamLauncher_x.y.z_aarch64.dmg` |
| macOS (Intel) | `ScamLauncher_x.y.z_x64.dmg` |
| Linux | `ScamLauncher_x.y.z_amd64.AppImage` (обновляется сам; `.deb` — нет) |

Лаунчер не подписан сертификатом, поэтому при первом запуске:

- **Windows:** «Windows защитила ваш компьютер» → «Подробнее» → «Выполнить в любом случае».
- **macOS:** перетащи в «Программы», запусти. Если macOS не даёт открыть — «Системные настройки» → «Конфиденциальность и безопасность» → «Всё равно открыть».
  Если пишет, что приложение «повреждено»: `xattr -cr /Applications/ScamLauncher.app`.
- **Linux:** `chmod +x ScamLauncher_*.AppImage` и запусти.

Дальше лаунчер обновляется сам — предложит при запуске.

## Разработка

Нужны Rust (stable) и Node.js 22+.

```bash
cd app && npm install
npm run tauri dev
```

Проверки как в CI:

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

## Выпуск версии

```bash
git tag v0.1.0 && git push origin v0.1.0
```

GitHub Actions соберёт установщики под Windows / macOS / Linux и `scam-pack` под все ОС,
подпишет обновления ключом из секретов `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
и опубликует релиз. Версию в файлах вручную менять не нужно — она берётся из тега.

> Приватный ключ апдейтера хранится только локально (`~/.tauri/scamlauncher.key`) и в секретах GitHub.
> Если его потерять, установленные лаунчеры перестанут принимать обновления.

## Структура

```
crates/scam-core   общая логика: конфиг, формат сборок, Яндекс Диск, синхронизация
crates/scam-mc     установка и запуск Minecraft
crates/scam-pack   CLI публикации сборок
app/               лаунчер (Tauri + React)
scam.config.json   ссылка на Я.Диск, OAuth ClientID, репозиторий GitHub
```

# Minecraft 1.20.1 — Rust Edition (Based on Horizon)

> **English** | [Русский](#minecraft-1201--rust-edition-на-базе-horizon)

A native Rust client, integrated server, and Vulkan (`wgpu`) renderer adapted (backported) from the open-source [Horizon](https://github.com/horizon-rust/horizon) engine (MIT License) to **Minecraft: Java Edition 1.20.1** (**Protocol 763**, **DataVersion 3465**).

## Key Features (1.20.1 Target)

- **Native Rust Client & Renderer**: Vulkan / `wgpu` rendering pipeline without requiring a Java runtime (`OpenJDK`) or external stub servers for single-player.
- **Minecraft 1.20.1 Registry & Data**: Native support for 1,003 blocks and 1,255 items (`legacy_1_20_1_registry`), including 1.20.1-era IDs (`minecraft:grass`, `minecraft:chain`, `minecraft:scute`), Bundle tooltip formatting, and Anvil/world data (`DataVersion 3465`).
- **Protocol 763**: Native 1.20.1 packet framing and multiplayer/integrated server compatibility (`horizon-protocol`, `horizon-1201-server`).
- **MIT Licensed**: Clean source tree with no proprietary Mojang assets included.

## Requirements

- [Rust toolchain](https://rustup.rs/) (stable)
- Python 3.10+ (for asset preparation and verification scripts)
- A local installation of **Minecraft: Java Edition 1.20.1** (used solely to copy your own local `1.20.1.jar` and assets into the git-ignored `.local/` folder)
- A Vulkan-capable GPU and up-to-date graphics drivers

## Build & Run

1. **Clone the repository:**
   ```bash
   git clone https://github.com/<your-username>/horizon-1.20.1-rust.git
   cd horizon-1.20.1-rust
   ```

2. **Build from source:**
   ```bash
   cargo build --release
   ```

3. **Prepare Minecraft 1.20.1 assets:**
   Make sure you have launched Minecraft 1.20.1 at least once in the official launcher, then run:
   ```bash
   python tools/prepare_1201.py
   ```
   If your `.minecraft` folder is in a custom location:
   ```bash
   python tools/prepare_1201.py --minecraft-dir "/path/to/.minecraft"
   ```
   This stages your local `1.20.1.jar` and assets into `.local/horizon-client/1.20.1` (excluded from Git).

4. **Launch:**
   - **Client:**
     ```bash
     cargo run --release -p horizon-client
     ```
   - **Launcher:**
     ```bash
     cargo run --release -p horizon-launcher
     ```

## Verification

- Check that no proprietary Mojang assets are tracked:
  ```bash
  python tools/check_public_tree.py
  ```
- Run workspace library tests:
  ```bash
  cargo test --workspace --lib
  ```

## License

This project is licensed under the **MIT License** — see [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
Based on [Horizon](https://github.com/horizon-rust/horizon).

---

# Minecraft 1.20.1 — Rust Edition (на базе Horizon)

Адаптация (бэкпорт) нативного Rust-клиента, встроенного сервера и рендерера Vulkan (`wgpu`) на базе открытого движка [Horizon](https://github.com/horizon-rust/horizon) (MIT License) под стандарт **Minecraft: Java Edition 1.20.1** (**Protocol 763**, **DataVersion 3465**).

## Основные возможности (стандарт 1.20.1)

- **Нативный Rust-клиент и рендерер**: рендеринг на Vulkan (`wgpu`) и работа одиночной игры без запуска Java (`OpenJDK`) и без внешних серверов-заглушек.
- **Реестр и данные 1.20.1**: нативная поддержка 1 003 блоков и 1 255 предметов (`legacy_1_20_1_registry`), корректные ID эпохи 1.20.1 (`minecraft:grass`, `minecraft:chain`, `minecraft:scute`), исправленный тултип `Bundle` и формат миров Anvil (`DataVersion 3465`).
- **Protocol 763**: поддержка сетевого протокола 1.20.1 (`horizon-protocol`, `horizon-1201-server`).
- **Лицензия MIT**: чистое дерево исходного кода без проприетарных файлов игры Mojang.

## Требования

- [Rust](https://rustup.rs/) (актуальная stable-версия)
- Python 3.10+ (для скрипта подготовки ресурсов и проверок)
- Локально установленная **Minecraft: Java Edition 1.20.1** (для копирования ваших локальных `1.20.1.jar` и ассетов в игнорируемую Git папку `.local/`)
- Видеокарта с поддержкой Vulkan и актуальные драйверы

## Сборка и запуск

1. **Клонируйте репозиторий:**
   ```bash
   git clone https://github.com/<your-username>/horizon-1.20.1-rust.git
   cd horizon-1.20.1-rust
   ```

2. **Соберите проект из исходников:**
   ```bash
   cargo build --release
   ```

3. **Подготовьте ресурсы Minecraft 1.20.1:**
   Убедитесь, что Minecraft 1.20.1 хотя бы один раз запускался в официальном лаунчере, затем выполните:
   ```bash
   python tools/prepare_1201.py
   ```
   Если папка `.minecraft` находится в нестандартном месте:
   ```bash
   python tools/prepare_1201.py --minecraft-dir "C:/путь/к/.minecraft"
   ```
   Скрипт подготовит `.local/horizon-client/1.20.1` из вашей локальной копии игры.

4. **Запуск:**
   - **Клиент:**
     ```bash
     cargo run --release -p horizon-client
     ```
   - **Лаунчер:**
     ```bash
     cargo run --release -p horizon-launcher
     ```

## Проверки

- Проверка чистоты публичного дерева (отсутствие файлов Mojang):
  ```bash
  python tools/check_public_tree.py
  ```
- Запуск юнит-тестов рабочего пространства:
  ```bash
  cargo test --workspace --lib
  ```

## Лицензия

Проект распространяется под лицензией **MIT** — подробности в файлах [LICENSE](LICENSE) и [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
Основано на открытом движке [Horizon](https://github.com/horizon-rust/horizon).

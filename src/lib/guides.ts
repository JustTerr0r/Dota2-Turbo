// Install help for optional dependencies (iteration 1: the user installs them by hand)

export interface Guide {
  title: string;
  why: string;
  steps: string[];
  command?: string;
  link?: { label: string; url: string };
}

const WINE_URL = "https://github.com/Gcenx/macOS_Wine_builds/releases/download/11.0_1/wine-stable-11.0_1-osx64.tar.xz";
const DXVK_URL =
  "https://github.com/Gcenx/DXVK-macOS/releases/download/v1.10.3-20230507/dxvk-macOS-async-v1.10.3-20230507.tar.gz";

const macGuides: Record<string, Guide> = {
  rosetta: {
    title: "Rosetta 2",
    why: "Wine работает как x86_64-приложение и запускается через Rosetta.",
    steps: ["Откройте Терминал и выполните команду ниже (может запросить пароль).", "Нажмите «Проверить снова»."],
    command: "softwareupdate --install-rosetta --agree-to-license",
  },
  wine: {
    title: "Wine",
    why: "Нужен, чтобы запустить resourcecompiler.exe из Workshop Tools Dota 2 (Windows-программа).",
    steps: [
      "Homebrew-cask Wine больше не устанавливается из-за Gatekeeper, поэтому ставим сборку Gcenx вручную.",
      "Откройте Терминал и выполните команду ниже: она скачает Wine Stable в ~/Applications и создаст ссылку ~/.local/bin/wine.",
      "Первый запуск wineboot создаёт окружение ~/.wine — это может занять минуту.",
      "Нажмите «Проверить снова».",
    ],
    command: [
      "mkdir -p ~/Applications ~/.local/bin",
      `curl -L -o /tmp/wine.tar.xz ${WINE_URL}`,
      "tar -xJf /tmp/wine.tar.xz -C ~/Applications",
      'ln -sf "$HOME/Applications/Wine Stable.app/Contents/Resources/wine/bin/wine" ~/.local/bin/wine',
      "~/.local/bin/wine wineboot -u",
    ].join(" && \\\n  "),
    link: { label: "Релизы Gcenx Wine", url: "https://github.com/Gcenx/macOS_Wine_builds/releases" },
  },
  dxvk: {
    title: "DXVK-macOS",
    why: "Встроенный Direct3D в Wine не может создать DirectX 11 устройство для resourcecompiler. DXVK решает это через Metal.",
    steps: [
      "Сначала установите Wine.",
      "Выполните команду ниже: она сохранит оригинальные DLL в ~/.wine/dxvk-backup, положит DLL DXVK в system32 и включит их через DllOverrides.",
      "Нажмите «Проверить снова».",
    ],
    command: [
      `curl -L ${DXVK_URL} | tar -xz -C /tmp`,
      "S=~/.wine/drive_c/windows/system32",
      "mkdir -p ~/.wine/dxvk-backup",
      "(cp -n $S/d3d11.dll $S/d3d10core.dll ~/.wine/dxvk-backup/ || true)",
      "cp /tmp/dxvk-macOS-async-v1.10.3-20230507/x64/{d3d11,d3d10core}.dll $S/",
      "~/.local/bin/wine reg add 'HKCU\\Software\\Wine\\DllOverrides' /v d3d11 /d native /f",
      "~/.local/bin/wine reg add 'HKCU\\Software\\Wine\\DllOverrides' /v d3d10core /d native /f",
    ].join(" && \\\n  "),
    link: { label: "Релизы DXVK-macOS", url: "https://github.com/Gcenx/DXVK-macOS/releases" },
  },
};

const workshopWhy = "resourcecompiler.exe собирает интерфейсную часть D2Turbo: рейтинг лиги и тиры героев. Без него Patch недоступен.";
const WORKSHOP_DOWNLOAD = {
  label: "Скачать resourcecompiler.zip",
  url: "https://github.com/Dota-Modding-Community/workshoptools/releases/latest/download/resourcecompiler.zip",
};
const workshopImport = "Распакуйте архив, нажмите «Импортировать папку» и выберите распакованную папку resourcecompiler.";

const workshopMac: Guide = {
  title: "Dota 2 Workshop Tools",
  why: workshopWhy,
  steps: [
    "В Steam на Mac такого DLC нет: Valve выпускает Workshop Tools только для Windows. Нужны Windows-файлы компилятора, они запускаются через Wine.",
    "Скачайте архив кнопкой ниже (около 70 МБ).",
    workshopImport,
  ],
  link: WORKSHOP_DOWNLOAD,
};

const workshopWindows: Guide = {
  title: "Dota 2 Workshop Tools",
  why: workshopWhy,
  steps: [
    "В Steam откройте Библиотеку, нажмите правой кнопкой на Dota 2 → Свойства → DLC.",
    "Отметьте «Dota 2 Workshop Tools DLC» и дождитесь окончания загрузки.",
    "Нажмите «Проверить снова».",
    `Если DLC нет в списке или он не скачивается, возьмите архив кнопкой ниже. ${workshopImport}`,
  ],
  link: WORKSHOP_DOWNLOAD,
};

/** Install help for the platform reported by the app (`macos`, `windows`). */
export function guidesFor(platform: string): Record<string, Guide> {
  if (platform === "windows") return { workshop: workshopWindows };
  return { ...macGuides, workshop: workshopMac };
}

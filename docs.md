# Combine

Десктопное приложение для автоматизации работы с Telegram-аккаунтами.

## Стек технологий

**Frontend:**
- React 19 + TypeScript 5.7
- Vite 6 (сборка)
- Tailwind CSS 3.4 + Radix UI (UI-компоненты)
- React Hook Form + Zod (валидация форм)
- Recharts (графики)
- Sonner (уведомления)

**Backend:**
- Tauri 2 (десктоп-фреймворк)
- Rust 2021 edition
- Tokio (асинхронный рантайм)
- rusqlite (SQLite, bundled)
- Криптография: AES, SHA2, SHA1, HMAC, MD5
- Сеть: ureq (HTTP + SOCKS-прокси)

## Структура проекта

```
combine/
├── src/                          # Frontend
│   ├── main.tsx                  # Точка входа
│   ├── App.tsx                   # Корневой компонент
│   ├── components/
│   │   ├── ui/                   # 47 Radix UI обёрток (button, dialog, form...)
│   │   ├── Dashboard.tsx         # Навигация, lazy-load страниц
│   │   ├── AccountsPage.tsx      # Управление аккаунтами
│   │   ├── CheckerPage.tsx       # Валидация аккаунтов
│   │   ├── MailingPage.tsx       # Массовая рассылка
│   │   ├── InviterPage.tsx       # Инвайтинг в группы/каналы
│   │   ├── ParserPage.tsx        # Парсинг участников
│   │   ├── WarmerPage.tsx        # Прогрев аккаунтов
│   │   ├── ConverterPage.tsx     # Конвертация сессий
│   │   ├── ClonerPage.tsx        # Клонирование каналов/групп
│   │   ├── ReporterPage.tsx      # Автоматические жалобы
│   │   ├── BoostPage.tsx         # Буст каналов
│   │   ├── StoriesPage.tsx       # Публикация сторис
│   │   ├── MasslookingPage.tsx   # Масслукинг
│   │   ├── AutoReplyPage.tsx     # Автоответчик
│   │   ├── FirstCommentPage.tsx  # Первый комментарий
│   │   ├── CreateBotsPage.tsx    # Создание ботов
│   │   ├── CreateChannelsPage.tsx# Создание каналов
│   │   ├── UsernameCheckerPage.tsx # Проверка юзернеймов
│   │   └── RandomizerPage.tsx    # Рандомизация текста
│   ├── hooks/
│   │   └── use-mobile.tsx        # Хук определения мобильного устройства
│   └── lib/
│       └── utils.ts              # Утилиты (cn, форматирование)
├── src-tauri/                    # Backend (Rust)
│   ├── Cargo.toml                # Зависимости Rust
│   ├── tauri.conf.json           # Конфигурация Tauri
│   └── src/
│       ├── main.rs               # Точка входа, регистрация команд
│       ├── accounts/             # Управление аккаунтами
│       │   ├── storage.rs        # Хранение сессий
│       │   ├── session.rs        # Работа с сессиями
│       │   ├── auth_login.rs     # Авторизация
│       │   ├── reauth.rs         # Переавторизация
│       │   ├── import.rs         # Импорт аккаунтов
│       │   ├── devices.rs        # Устройства
│       │   ├── geo.rs            # Геолокация
│       │   ├── aging.rs          # Прогрев/старение
│       │   ├── actions.rs        # Массовые действия
│       │   └── commands.rs       # Tauri-команды
│       ├── mtproto/              # Telegram MTProto протокол
│       │   ├── client.rs         # MTProto-клиент
│       │   ├── auth.rs           # Аутентификация
│       │   ├── transport.rs      # Сетевой транспорт
│       │   ├── crypto.rs         # Криптография
│       │   ├── tl.rs             # Type Language схема
│       │   ├── tl_gen.rs         # Кодогенерация TL
│       │   ├── invite.rs         # Обработка инвайтов
│       │   └── text_parse.rs     # Парсинг сообщений
│       ├── checker/              # Валидация аккаунтов
│       │   ├── runner.rs         # Запуск проверок
│       │   ├── checks.rs         # Проверки (2FA, возраст...)
│       │   ├── validate.rs       # Логика валидации
│       │   ├── analysis.rs       # Анализ результатов
│       │   ├── nft.rs            # NFT-превью
│       │   └── commands.rs       # Tauri-команды
│       ├── queue/                # Очередь задач
│       ├── proxy/                # Управление прокси
│       ├── mailing/              # Рассылка
│       ├── inviter/              # Инвайтинг
│       ├── parser/               # Парсинг + user lookup
│       ├── warmer/               # Прогрев
│       ├── converter/            # Конвертация (telethon, pyro, tdata)
│       ├── cloner/               # Клонирование
│       ├── reporter/             # Жалобы
│       ├── boost/                # Буст
│       ├── stories/              # Сторис
│       ├── masslooking/          # Масслукинг
│       ├── auto_reply/           # Автоответ
│       ├── first_comment/        # Первый комментарий
│       ├── username_checker/     # Проверка юзернеймов
│       ├── randomizer/           # Рандомизация
│       ├── botcreator/           # Создание ботов
│       ├── channelcreator/       # Создание каналов
│       ├── llm/                  # Интеграция с LLM (OpenAI-совместимый)
│       ├── settings.rs           # Настройки приложения
│       └── debug.rs              # Отладка
├── package.json
├── vite.config.ts
├── tsconfig.json
├── tailwind.config.ts
├── postcss.config.js
└── index.html
```

## Модули и функциональность

### Аккаунты
- Импорт из файлов, auth-ключей, различных форматов сессий
- Авторизация и переавторизация
- Управление устройствами и геолокацией
- Массовые действия (удаление, смена настроек)
- Прогрев/старение аккаунтов

### Чекер (валидация)
- Проверка статуса аккаунта (бан, ограничения)
- Проверка 2FA, возраста аккаунта
- Подсчёт каналов/групп
- NFT-превью аватарок
- Сортировка и анализ результатов

### Рассылка (Mailing)
- Массовая отправка сообщений
- Настройка задержек и потоков
- Интеграция с LLM для генерации текста

### Инвайтер
- Приглашение пользователей в группы/каналы
- Настройка лимитов и пауз

### Парсер
- Парсинг участников групп/каналов
- Поиск пользователей (User Lookup)

### Конвертер сессий
- Telethon → внутренний формат
- Pyrogram → внутренний формат
- tdata → внутренний формат

### Клонер
- Копирование контента каналов/групп

### Прокси
- SOCKS-прокси
- Валидация прокси
- Автораспределение по аккаунтам

### Очередь задач
- Семафор: максимум 5 одновременных задач
- Статусы: Queued → Running → Done/Failed/Stopped
- Остановка отдельных задач

## Конфигурация сборки

### Vite
- Code splitting: вендор-чанки (recharts, radix-ui, tauri, forms, phone, dates, carousel, ui, react)
- Рандомизированные имена ассетов в продакшене (только хэш, без имён компонентов)
- Path alias: `@/*` → `./src/*`

### Tauri
- Фиксированное окно: 1504×871, без ресайза
- CSP настроена для WebView
- Идентификатор: `com.combine.app`

### TypeScript
- Strict mode
- Path alias через tsconfig

## Команды

```bash
# Разработка
npm run dev          # Запуск Vite dev-server
npm run tauri dev    # Запуск Tauri в режиме разработки

# Сборка
npm run build        # Сборка фронтенда
npm run tauri build  # Сборка десктоп-приложения

# Линтинг
npm run lint         # ESLint
```

## Архитектурные решения

1. **Lazy loading** — страницы загружаются при первом посещении, остаются в памяти
2. **IPC** — ~60+ Tauri-команд для связи фронта и бэка
3. **MTProto** — кастомная реализация протокола Telegram (не используются сторонние библиотеки)
4. **SQLite** — локальная БД для хранения аккаунтов, сессий, настроек
5. **Очередь с семафором** — ограничение параллелизма до 5 задач
6. **LLM-интеграция** — OpenAI-совместимый API для генерации/рандомизации текста
7. **Дедупликация сессий** — очистка при запуске + периодическая (60 сек)

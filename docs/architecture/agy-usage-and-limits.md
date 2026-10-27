# Antigravity CLI (`agy`) Usage, Limits & Monitoring Architecture

В данном документе описана архитектура хранения и отслеживания потребления токенов, контекста и лимитов в **Google Antigravity CLI (`agy`)**, а также спецификация для их сбора во внешних приложениях и демонах мониторинга (включая `ssh-app` / `usage_probe.py`).

---

## 1. Обзор архитектуры хранения данных в `agy`

Все рабочие файлы и конфигурации CLI находятся в каталоге:
`~/.gemini/antigravity-cli/`

В отличие от Claude Code или OpenAI Codex, `agy` написан на языке **Go** и разделяет учет на два уровня:
1. **Локальная история сессии (Token Consumption & Context Window):** сохраняется на диск пошагово в структурированном виде.
2. **Серверные квоты аккаунта (Account Plan Quotas):** запрашиваются через RPC-эндпоинт бекенда Google и хранятся в памяти запущенного процесса.

---

## 2. Локальные файлы и их структура

### 2.1. Транскрипт шагов и учет токенов (`transcript.jsonl`)
Каждая сессия имеет уникальный UUID (`conversation_id`). Пошаговый лог диалога и использования токенов находится по пути:
```text
~/.gemini/antigravity-cli/brain/<conversation-id>/.system_generated/logs/transcript.jsonl
```
Каждый ответ модели регистрируется строкой JSON типа `PLANNER_RESPONSE`:
```json
{
  "step_index": 108,
  "source": "MODEL",
  "type": "PLANNER_RESPONSE",
  "status": "DONE",
  "created_at": "2026-10-18T12:07:59Z",
  "input_tokens": 3635,
  "cache_read_tokens": 108266,
  "output_tokens": 139,
  "tool_calls": [...]
}
```
* **Использованный контекст (Context Used):**
  $$\text{context\_used} = \text{input\_tokens} + \text{cache\_read\_tokens} + \text{output\_tokens}$$
* **Лимит окна контекста (Context Limit):**
  * `Gemini Flash`: 1 000 000 токенов
  * `Gemini Pro`: 2 000 000 токенов

### 2.2. Связка рабочей директории с сессией (`last_conversations.json`)
Чтобы сопоставить текущую рабочую директорию tmux-панели (`pane_current_path`) с активной сессией `agy`:
* **Файл:** `~/.gemini/antigravity-cli/cache/last_conversations.json`
* **Формат:**
  ```json
  {
    "/Users/dev/rust/ssh-app": "9fcd4e98-4e17-453c-99a1-4600828e6d64",
    "/Users/dev/rust/ide": "be84188f-482f-46b7-a699-05c2b9025ae5"
  }
  ```

### 2.3. Активные процессы (`presence/`)
Запущенные сессии удерживают advisory lock:
* **Файлы:** `~/.gemini/antigravity-cli/presence/<conversation-id>.lock`

### 2.4. Текущая модель (`settings.json`)
* **Файл:** `~/.gemini/antigravity-cli/settings.json`
* Поле: `"model"` (например, `"Gemini 3.8 Flash (Medium)"` или `"Gemini 2.5 Pro"`).

---

## 3. Серверные квоты и лимиты тарифа (Account Quotas)

### 3.1. Как `agy` запрашивает квоты
Внутри бинарника `agy` вызов `doRefreshQuota` выполняет запрос Connect-RPC / gRPC к сервису Google Cloud Code:
```text
POST https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary
Headers:
  Authorization: Bearer <OAuth-token>
  Content-Type: application/json
```
Авторизационный токен хранится в:
`~/.gemini/antigravity-cli/antigravity-oauth-token`

### 3.2. Хранение квоты
Полученный ответ кэшируется в оперативной памяти процесса (`*store.Manager.doRefreshQuota`) и используется для отображения строки статуса:
`%s in | %s out | %s thinking`
На диск файл с остатком плана (в отличие от токенов сессии) штатно не выгружается.

### 3.3. Фиксация исчерпания лимитов
Когда у пользователя заканчивается квота или срабатывает рейт-лимит, сервер возвращает ошибку, которую `agy` фиксирует:
1. В последней строке `transcript.jsonl`: поле `"status": "ERROR"` с текстом ошибки в поле `"content"`.
2. В файле логов: `~/.gemini/antigravity-cli/cli.log`.

Характерные сигнатуры ошибок:
* `individual quota reached`
* `MODEL_CAPACITY_EXHAUSTED`
* `RESOURCE_EXHAUSTED`
* `HTTP 429` / `rate limit exceeded`
* `Overloaded prefill queue`

---

## 4. Спецификация для сбора данных в `usage_probe.py` (`ssh-app`)

Для интеграции мониторинга сессий `agy` в `ssh-app` в скрипт `crates/core/src/usage_probe.py` добавляется функция `agy_state(cwd)`:

```python
def agy_state(cwd):
    """Считывает context_used, context_limit, модель и статус лимитов для agy."""
    cache_path = os.path.expanduser("~/.gemini/antigravity-cli/cache/last_conversations.json")
    try:
        with open(cache_path, "r", encoding="utf-8") as f:
            mapping = json.load(f)
    except Exception:
        return {}

    conv_id = mapping.get(os.path.realpath(cwd)) or mapping.get(cwd)
    if not conv_id:
        return {}

    transcript_path = os.path.expanduser(
        f"~/.gemini/antigravity-cli/brain/{conv_id}/.system_generated/logs/transcript.jsonl"
    )
    if not os.path.exists(transcript_path):
        return {}

    out = {}

    # 1. Извлекаем выбранную модель
    try:
        settings_path = os.path.expanduser("~/.gemini/antigravity-cli/settings.json")
        with open(settings_path, "r", encoding="utf-8") as sf:
            settings = json.load(sf)
            out["model"] = settings.get("model", "Gemini")
    except Exception:
        out["model"] = "Gemini"

    # Контекстный лимит зависит от модели (Flash = 1M, Pro = 2M)
    out["context_limit"] = 2_000_000 if "pro" in str(out.get("model")).lower() else 1_000_000

    # 2. Сканируем последние строки транскрипта
    for line in reversed(tail_lines(transcript_path, limit=64 * 1024)):
        try:
            entry = json.loads(line)
        except Exception:
            continue

        # Последний подсчет токенов
        if "context_used" not in out and entry.get("type") == "PLANNER_RESPONSE":
            inp = entry.get("input_tokens") or 0
            cache = entry.get("cache_read_tokens") or 0
            outp = entry.get("output_tokens") or 0
            if inp or cache or outp:
                out["context_used"] = inp + cache + outp

        # Проверка на предупреждения/ошибки исчерпания лимитов
        if "limit_note" not in out and entry.get("status") == "ERROR":
            err_msg = entry.get("content", "")
            if re.search(r"quota|capacity|rate.limit|429|exhausted|overloaded", err_msg, re.I):
                out["limit_note"] = err_msg[:200].strip()

        if "context_used" in out and "limit_note" in out:
            break

    return out
```

### 4.1. Подключение в цикл обхода tmux-сессий:
В главном цикле `usage_probe.py`:
```python
    s = {"session": name, "agent": agent}
    if agent == "claude":
        s.update(claude_state(cwd))
    elif agent == "codex":
        s["limits"] = codex_limits()
    elif agent == "agy":
        s.update(agy_state(cwd))
    elif agent == "ollama" and arg:
        s["ollama_model"] = arg
    sessions.append(s)
```

---

## 5. Реализация (2026-10-18)

`agy_state` добавлена в `crates/core/src/usage_probe.py` и проверена на реальных файлах этого Mac: модель, `context_used` и `context_limit` определяются, в полоске ssh-app показываются модель и заполнение контекста. Отличия от наброска выше: транскрипт читается с хвоста 1 МБ (строки `PLANNER_RESPONSE` бывают большими), `conversation_id` проверяется регулярным выражением, ошибка лимита учитывается только за последние 6 часов.

Намеренно **не** сделано: запрос `retrieveUserQuotaSummary` с OAuth-токеном `agy`. Это внутренний эндпоинт Google, а токен принадлежит самому `agy`; приложение не должно им пользоваться. Поэтому для `agy` реальных процентов тарифа нет, только контекст и сообщение об исчерпании лимита. Размеры окон (Flash 1M, Pro 2M) взяты из этого документа, а не из самого инструмента.

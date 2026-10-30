app-title = Agentdeck

nav-instances = Инстансы
nav-drift = Расхождения
nav-plugins = Плагины
nav-processes = Процессы

loading = Загрузка…
load-error = Не удалось загрузить: { $what }

instances-title = Инстансы
instances-empty = В реестре пока нет инстансов.
instances-add = Добавить
instances-remove = Удалить
instances-quotas = Квоты
instances-standalone = Другие источники расхода
quotas-no-data = Данных об использовании пока нет.
status-enabled = включён
status-disabled = выключен
t3-label = T3
t3-unavailable = T3 недоступен
threads-count =
    { $count ->
        [one] { $count } поток
        [few] { $count } потока
        [many] { $count } потоков
       *[other] { $count } потоков
    }

account-ok = ок
account-stale = устаревшие
account-error = ошибка
account-waiting = ожидание
quota-in-hours = сброс через { $h } ч { $m } мин
quota-in-mins = сброс через { $m } мин

drift-title = Расхождения
drift-no-drift = Расхождений нет.
drift-apply = Применить
drift-import = Импортировать
drift-import-server = Импортировать { $server }
drift-fix = Исправить (сначала бэкап)
drift-reenable = Переподключить
drift-confirm = Подтвердить
drift-confirm-replace = Подтвердить замену { $path }
drift-cancel = Отмена
drift-preview = Предпросмотр
drift-working = Выполняется…
drift-group-t3 = Инстансы провайдера T3
drift-not-in-registry = Нет в реестре.
plugins-title = Плагины
processes-title = Процессы
processes-empty = Нет запущенных процессов агентов.
processes-stop = Остановить
processes-kind-agent = сессия
processes-kind-t3 = T3-скоуп
processes-kind-harness = раннер
processes-confirm-title = Остановить { $title }?
processes-confirm-will = Будут остановлены эти процессы:
processes-force = Принудительно завершить (SIGKILL) после 5 с ожидания
processes-confirm-ok = Остановить
processes-cancel = Отмена
processes-stop-terminated = остановлены
processes-stop-killed = принудительно остановлены
processes-stop-exited = уже завершены
processes-stop-running = всё ещё работают после ожидания
processes-stop-result = { $title }: { $what }
processes-t3-note = Сессии T3 прерываются через сам T3, когда он доступен, а не завершаются сигналом.
coming-soon = Появится в следующих юнитах.

plugins-empty = Нет доступных плагинов.
plugins-enable = Включить
plugins-disable = Выключить
plugins-confirm = Подтвердить
plugins-cancel = Отмена
plugins-enabled = включён
plugins-disabled = выключен
plugins-unavailable = недоступен (нужен T3)
plugins-preview-enable = Включение { $name } выполнит:
plugins-preview-disable = Выключение { $name } уберёт его файлы (данные сохранятся):
add-title = Новый инстанс
add-back = Назад к инстансам
add-harness = Harness
add-harness-note = opencode и agy появятся в M7.
add-id = ID инстанса
add-id-note = ID нельзя переименовать: к ним привязана история T3.
add-display-name = Отображаемое имя
add-home = Домашний каталог
add-home-use-suggestion = Подставить
add-preview = Предпросмотр
add-apply = Применить
add-working = Работаю…
add-login-title = Вход
add-login-start = Начать вход
add-login-device-auth = Вход по коду устройства (codex)
add-login-code = Вставленный код
add-login-send = Отправить
add-login-cancel = Отмена
add-login-copy = Копировать
add-login-open = Открыть
add-login-cancelled = Вход отменён.
add-login-done = Процесс входа завершился с кодом 0.
add-login-failed = Ошибка входа: { $reason }.
add-login-retry-device = Порт 1455 занят; повторить через код устройства
add-login-device-code = Код устройства: { $code }
add-open-failed = Не удалось открыть браузер; скопируйте ссылку.
add-wrapper-title = Обёртка терминала (необязательно)
add-t3-title = Синхронизация с T3 (необязательно)

remove-ask = Удалить { $id }?
remove-confirm = Удалить
remove-cancel = Оставить
remove-keep-home = Каталог входа сохраняется.
remove-threads =
    { $count ->
        [one] У T3 остался { $count } поток этого id; история сохранится, но не запустится.
        [few] У T3 осталось { $count } потока этого id; история сохранится, но не запустится.
        [many] У T3 осталось { $count } потоков этого id; история сохранится, но не запустится.
       *[other] У T3 осталось { $count } потоков этого id; история сохранится, но не запустится.
    }

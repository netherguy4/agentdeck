app-title = Agentdeck

nav-instances = Инстансы
nav-drift = Расхождения
nav-plugins = Плагины
nav-processes = Процессы

loading = Загрузка…
load-error = Не удалось загрузить: { $what }

instances-title = Инстансы
instances-empty = В реестре пока нет инстансов.
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

drift-title = Расхождения
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

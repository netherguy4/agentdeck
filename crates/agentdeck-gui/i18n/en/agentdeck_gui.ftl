app-title = Agentdeck

nav-instances = Instances
nav-drift = Drift
nav-plugins = Plugins
nav-processes = Processes

loading = Loading…
load-error = Could not load: { $what }

instances-title = Instances
instances-empty = No instances in the registry yet.
instances-quotas = Quotas
instances-standalone = Other usage sources
quotas-no-data = No usage data yet.
status-enabled = enabled
status-disabled = disabled
t3-label = T3
t3-unavailable = T3 unavailable
threads-count =
    { $count ->
        [one] { $count } thread
       *[other] { $count } threads
    }

account-ok = ok
account-stale = stale
account-error = error
account-waiting = waiting

drift-title = Drift
plugins-title = Plugins
processes-title = Processes
processes-empty = No agent processes running.
processes-stop = Stop
processes-kind-agent = session
processes-kind-t3 = T3 scope
processes-kind-harness = harness
processes-confirm-title = Stop { $title }?
processes-confirm-will = These processes will be stopped:
processes-force = Force kill (SIGKILL) after a 5 s grace period
processes-confirm-ok = Stop now
processes-cancel = Cancel
processes-stop-terminated = stopped
processes-stop-killed = force-stopped
processes-stop-exited = already exited
processes-stop-running = still running after the grace period
processes-stop-result = { $title }: { $what }
processes-t3-note = T3 sessions are interrupted through T3 when it is reachable, not killed.
coming-soon = Coming in a later unit.

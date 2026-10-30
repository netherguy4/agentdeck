app-title = Agentdeck

nav-instances = Instances
nav-drift = Drift
nav-plugins = Plugins
nav-processes = Processes

loading = Loading…
load-error = Could not load: { $what }

instances-title = Instances
instances-empty = No instances in the registry yet.
instances-add = Add
instances-remove = Remove
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
drift-no-drift = No drift.
drift-apply = Apply
drift-import = Import
drift-import-server = Import { $server }
drift-fix = Fix (backup first)
drift-reenable = Re-enable
drift-confirm = Confirm
drift-confirm-replace = Confirm replace { $path }
drift-cancel = Cancel
drift-preview = Preview
drift-working = Working…
drift-group-t3 = T3 provider instances
drift-not-in-registry = Not in the registry.
plugins-title = Plugins
processes-title = Processes
coming-soon = Coming in a later unit.

plugins-empty = No plugins available.
plugins-enable = Enable
plugins-disable = Disable
plugins-confirm = Confirm
plugins-cancel = Cancel
plugins-enabled = enabled
plugins-disabled = disabled
plugins-unavailable = unavailable (needs T3)
plugins-preview-enable = Enabling { $name } will:
plugins-preview-disable = Disabling { $name } will remove its files (data is kept):
add-title = Add instance
add-back = Back to Instances
add-harness = Harness
add-harness-note = opencode and agy arrive in M7.
add-id = Instance id
add-id-note = Ids are never renamed: T3 history binds to them.
add-display-name = Display name
add-home = Home directory
add-home-use-suggestion = Use suggestion
add-preview = Preview
add-apply = Apply
add-working = Working…
add-login-title = Login
add-login-start = Start login
add-login-device-auth = Use device auth (codex)
add-login-code = Pasted code
add-login-send = Send
add-login-cancel = Cancel
add-login-copy = Copy
add-login-open = Open
add-login-cancelled = Login cancelled.
add-login-done = Login process exited 0.
add-login-failed = Login failed: { $reason }.
add-login-retry-device = Port 1455 is busy; retry with device auth
add-login-device-code = Device code: { $code }
add-open-failed = Could not open the browser; copy the URL instead.
add-wrapper-title = Terminal wrapper (optional)
add-t3-title = Sync to T3 (optional)

remove-ask = Remove { $id }?
remove-confirm = Remove
remove-cancel = Keep
remove-keep-home = The login dir is kept.
remove-threads =
    { $count ->
        [one] T3 keeps { $count } thread for this id; history stays but cannot launch.
       *[other] T3 keeps { $count } threads for this id; history stays but cannot launch.
    }

# Project configuration

A spackle project is defined by a `spackle.toml` file at the root directory. Below is a reference for the configuration file.

### Field legend

<span style="color: darkseagreen;">{s}</span> = slot environment (`{{ }}` will be replaced by slot values)

### Templating syntax

Slot environments — `.j2` / `.tera` file contents, file names, and <span style="color: darkseagreen;">{s}</span> fields — are rendered with [Tera](https://keats.github.io/tera/docs/). See the Tera docs for the full template syntax (variables, filters, conditionals, loops, etc.).

### Global slots

Global slots are available in all slot environments (`.j2` / `.tera` file contents, file names, <span style="color: darkseagreen;">{s}</span> fields).

- `_project_name` `string`
  - The name of the project itself
- `_output_name` `string`
  - The name of the output directory

## Project-level config

### name `string`

The name of the project. This also sets the `_project_name` global slot, so keep that in mind. If this isn't set, the project name will be inferred from the directory name.

```toml
name = "my_cool_project"
```

### ignore `string[]`

Files and directories to ignore when copying. These will be relative to the project directory.

```toml
ignore = [
    ".git"
]
```

## slots `table`

Slots are defined by one or more `[[slots]]` table entries in the `spackle.toml` file.

```toml
[[slots]]
key = "slot_name"
type = "String"
name = "Slot name"
description = "A description of the slot"
default = "default value"
```

### key `string`

The key of the slot in the project. This is the identifier you can use in slot environments to retrieve the value of the slot.

```toml
key = "slot_name"
```

### type `string`

The data type of the slot. Can be one of the following:

- `String`
- `Number`
- `Boolean`

```toml
type = "String"
```

### needs `string[]`

The slots that the slot depends on.

```toml
needs = ["some_slot", "other_slot"]
```

### name `string`

The human-friendly name of the slot.

```toml
name = "Slot name"
```

### description `string`

The human-friendly description of the slot.

```toml
description = "A description of the slot"
```

### default `string`

The default value of the slot. The CLI will use the default value if one is not provided by the user (e.g. they press enter without typing anything).

For library consumers, is up to you to decide whether to use the default value or not. The generate function will not use the default value if the slot is not provided, and will instead error if a slot is not provided properly.

```toml
default = "default value"
```

## hooks `table`

Hooks are defined by one or more `[[hooks]]` table entries in the `spackle.toml` file. Hooks are ran after the project is rendered and ran in the generated directory, and can be used to modify the project or enable specific functionality.

```toml
[[hooks]]
name = "create file"
command = ["touch", "new_file"]
optional = { default = true }
needs = ["foo"]
if = "{{foo}} != 'bar'"
name = "Create a new file"
description = "Create a new file called new_file"
```

#### Command forms and substitution

Every hook runs under `bash -c`, so chaining commands works without any special handling. `bash` must be on `PATH` at hook-execution time (Linux and macOS). `command` accepts two forms, which differ in how templated slot values are substituted:

**String form** — templated as a whole, then run via `bash -c`. Slot values substitute as **raw shell text**, so you own quoting. Use this when you want shell semantics from a slot value, or just for readability:

```toml
command = "touch new_file && chmod +x new_file"

# Raw substitution — quote it yourself if the value should stay literal:
command = "echo '{{ name }}' && echo done"
```

**Array form** — each element is templated, then POSIX-quoted (bare shell operators `&&`, `||`, `|`, `;` pass through), then joined and run via `bash -c`. Slot values become **literal arguments** and can never act as shell syntax:

```toml
command = ["touch", "new_file", "&&", "chmod", "+x", "new_file"]

# Whitespace and metacharacters in a value are quoted automatically:
command = ["git", "commit", "-m", "initial commit", "&&", "git", "status"]
# runs: git commit -m 'initial commit' && git status

# A slot value can't break out — it stays one argument:
command = ["touch", "{{ name }}"]
# name = "weird; rm x"  →  touch 'weird; rm x'   (one file, no injection)
```

An array of the shape `["bash"|"sh", "-c", body]` is treated as the string form: `body` is templated and run via `bash -c`.

##### Blocked patterns

Hooks run as the invoking user, so the blast radius is whatever that user could already type at their shell. spackle still refuses a small denylist of unambiguously catastrophic rendered commands — a fork bomb, or a recursive force-remove of `/` or a top-level system directory (`rm -rf /`, `rm -rf /etc`, …). These are caught at plan time and the hook does not run.

### key `string`

The identifier for the hook.

### command `string` or `string[]` <span style="color: darkseagreen;">{s}</span>

The command to execute, in either string or array form (see [Command forms and substitution](#command-forms-and-substitution)). Accepts values from slots. Runs under `bash -c`.

```toml
command = "echo Hello {{ foo }}"
# or
command = ["echo", "Hello {{ foo }}"]
```

### default `boolean`

The default value of the hook. The CLI will use the default value if one is not provided by the user (e.g. they press enter without typing anything).

```toml
default = false
```

### needs `string[]`

The items on which the hook depends. The hook will only be executed if all the dependencies are satisfied. A dependency is satisfied if the dependency is enabled and all of its own dependencies are satisfied.

A slot is considered enabled if it has a non-default value (default values include `""`, `0`, and `false` for example).

> Note: Because `if` is evaluated only on hook run time, it is not taken into account when determining satisfaction of `needs`.

```toml
needs = ["some_hook", "other_slot"]
```

### if `string` <span style="color: darkseagreen;">{s}</span>

The condition on which to execute the hook. Accepts values from slots.

```toml
if = "{{ foo }} != 'bar'"
```

> Note: The `if` condition is evaluated directly before the hook is executed.

#### Dependencies on other hooks

If you want to run a hook only if another hook has already been run, you can use the `hook_ran_{hook_key}` variable.

```toml
if = "{{ hook_ran_other_hook }}"
```

### name `string`

The name of the hook.

### description `string`

A description for the hook.

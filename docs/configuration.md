# Project configuration

A spackle project is defined by a `spackle.toml` file at the root directory. Below is a reference for the configuration file.

### Field legend

<span style="color: darkseagreen;">{s}</span> = slot environment (`{{ }}` will be replaced by slot values)

### Universal slots

Universal slots are available in all slot environments (`.j2` file contents, file names, <span style="color: darkseagreen;">{s}</span> fields).

- `_project_name` `string`
  - The name of the output directory

## Project-level config

### name `string`

The name of the project. This also sets the `project_name` universal slot, so keep that in mind. If this isn't set, the project name will be inferred from the directory name.

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
type = "string"
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

#### Command sequences

To manage hook command sequences, create a single hook that runs a shell command, invoking your desired commands in sequence. For example:

```toml
[[hooks]]
key = "create_file"
command = ["bash", "-c", "touch new_file && chmod +x new_file"]
```

### key `string`

The identifier for the hook.

### command `string[]` <span style="color: darkseagreen;">{s}</span>

The command to execute. The first element is the command and the rest are arguments. Accepts values from slots.

```toml
command = ["echo", "Hello {{ foo }}"]
```

### optional `{ default = bool }`

When defined, the user can toggle the hook. `default` describes the default state of the hook.

```toml
optional = { default = true }
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

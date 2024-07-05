# Project configuration

A spackle project is defined by a `spackle.toml` file at the root directory. Below is a reference for the configuration file.

### Field legend

<span style="color: darkseagreen;">{s}</span> = slot environment (`{{ }}` will be replaced by slot values)

### Universal slots

Universal slots are available in all slot environments (`.j2` file contents, file names, <span style="color: darkseagreen;">{s}</span> fields).

- project_name `string`
  - The name of the project, defined by the name of the output directory

## Project-level config

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

## hooks `table`

Hooks are defined by one or more `[[hooks]]` table entries in the `spackle.toml` file. Hooks are ran after the project is rendered and ran in the generated directory, and can be used to modify the project or enable specific functionality.

```toml
[[hooks]]
name = "create file"
command = ["touch", "new_file"]
optional = { default = true }
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

### if `string` <span style="color: darkseagreen;">{s}</span>

The condition to execute the hook. Accepts values from slots.

```toml
if = "{{ foo }} != 'bar'"
```

#### Dependencies on other hooks

If you want to run a hook only if another hook has already been run, you can use the `hook_ran_{hook_key}` variable.

```toml
if = "{{ hook_ran_other_hook }}"
```

### name `string`

The name of the hook.

### description `string`

A description for the hook.

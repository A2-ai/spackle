# Project configuration

A spackle project is defined by a `spackle.toml` file at the root directory. Below is a reference for the configuration file.

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

The key of the slot in the project. This is the variable name in the template.

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
if = "{{foo}} != 'bar'"
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

### command `string[]`

The command to execute. The first element is the command and the rest are arguments.

### optional

When defined, the user can toggle the hook. `default` describes the default state of the hook.

```
optional = { default = true }
```

### if `string`

The condition to execute the hook. The condition is templated just like the template files, allowing you to conditionally execute a hook.

### name `string`

The name of the hook.

### description `string`

A description for the hook.


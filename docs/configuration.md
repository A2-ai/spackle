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

Hooks are defined by one or more `[[hooks]]` table entries in the `spackle.toml` file.

```toml
[[hooks]]
name = "create file"
command = ["touch", "new_file"]
if = "{{foo}} != 'bar'"
```

### name `string`

The name of the hook.

### command `string[]`

The command to execute. The first element is the command and the rest are arguments. The command is executed in the generated directory.

### if `string`

The condition to execute the hook. The condition is evaluated in the context of the project slots.

## slots `table`

Slots are defined by one or more `[[slots]]` table entries in the `spackle.toml` file.

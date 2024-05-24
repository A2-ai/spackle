# 🚰 spackle

A spackle project is composed of a `spackle.toml` file at the root containing a list of slots.

## Slot config

```toml
[[slots]]
key = "slot_name"
type = "string"
name = "Slot name"
description = "A description of the slot"
```

### key `string`

The key of the slot in the project. This is the variable name in the template.

### type `string`

The type of the slot. Can be one of the following:

- `String`
- `Number`
- `Boolean`

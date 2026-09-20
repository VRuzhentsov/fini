# SettingsListGroup

Wrapper for grouped Settings rows.

## Purpose

Render adjacent [[SettingsListItem]] rows as one visual group with shared rounded corners and internal separators.

## Layout

- First row gets top rounding
- Last row gets bottom rounding and no bottom divider
- Children remain responsible for their own one-column or start/end content

## Theme

Uses existing DaisyUI/Tailwind tokens. It does not accept or inspect the current theme.

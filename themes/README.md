# Orca Themes

Built-in color themes for Orca. Each `.css` file overrides Orca's generated color
palette by redefining the `@define-color` variables used throughout the UI.

## Using a theme

Copy any `.css` file to `$XDG_DATA_HOME/orca/themes/active.css`. Orca loads
`active.css` at startup and applies it on top of the generated palette.

## Writing a custom theme

Define any subset of the variables below. Unset variables keep the generated
defaults from the active color scheme.

```css
@define-color orca_bg      #1e1e2e;
@define-color orca_surface #313244;
@define-color orca_text    #cdd6f4;
@define-color orca_subtext #a6adc8;
@define-color orca_accent  #f5c2e7;
```

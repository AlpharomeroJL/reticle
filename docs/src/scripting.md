# Scripting

`reticle-script` embeds a scripting language so a layout can be built, queried, and
transformed programmatically.

## The API

The engine is `rhai`, a small embeddable Rust scripting language. Scripts get an
API over the model: create cells and shapes, query and transform geometry, run the
design-rule checker, invoke the router, and export to a file format. Because the
API is the same model the application edits, a script and a hand edit are
interchangeable.

## Plugins and examples

A plugin folder lets a script extend the application, and a set of worked example
scripts shows the common tasks: generating a parameterized cell, sweeping a rule
threshold, batch-checking a directory of layouts, and scripting an export pipeline.

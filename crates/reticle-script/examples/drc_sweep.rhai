// DRC sweep.
//
// Loads a small technology (declaring width and spacing rules), draws a cell that
// deliberately violates them, then sweeps DRC and reports what it found. Use it as
// a template for regression-checking a rule deck against known-bad geometry.
//
// Assumes a "PIXEL" cell already exists if run after param_cell.rhai in the same
// plugin directory; otherwise it creates its own test cell.

// -------- technology with rules --------
// metal1 (1/0): minimum width 100, minimum spacing 140.
let loaded = load_technology(
    "technology sweep_demo\n" +
    "dbu_per_micron 1000\n" +
    "layer 1 0 metal1 4488FFFF\n" +
    "rule width   1 0 100\n" +
    "rule spacing 1 0 140\n"
);
print("rules loaded: " + loaded);

// -------- deliberately non-compliant geometry --------
if !has_cell("DRC_TEST") {
    create_cell("DRC_TEST");
}
// A sliver 50 DBU wide -> violates the 100 width rule.
add_rect("DRC_TEST", 1, 0, 0, 0, 50, 1000);
// A second wire only 60 DBU away -> violates the 140 spacing rule.
add_rect("DRC_TEST", 1, 0, 110, 0, 400, 1000);

// -------- sweep --------
let violations = run_drc("DRC_TEST");
print("DRC violations: " + violations);
for msg in drc_messages("DRC_TEST") {
    print("  - " + msg);
}

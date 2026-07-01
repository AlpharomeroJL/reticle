// Parameterized cell generator.
//
// Builds a leaf "PIXEL" cell, then tiles it into a "SENSOR" top cell as a
// rows x columns array. Change the parameters below to resize the sensor; the
// script is otherwise data-driven.
//
// Run it with:
//     let mut engine = ScriptEngine::new();
//     engine.run_plugin_dir("crates/reticle-script/examples")?;   // or eval this file
//
// Layers used (layer/datatype):
//   metal1 = 1/0    (the pixel body)
//   via1   = 2/0    (a contact inside each pixel)

// -------- parameters --------
let pixel_w   = 800;    // pixel width  in DBU
let pixel_h   = 800;    // pixel height in DBU
let via       = 200;    // square via size in DBU
let columns   = 8;
let rows      = 6;
let pitch_x   = 1000;   // column pitch in DBU
let pitch_y   = 1000;   // row pitch in DBU

// -------- leaf cell --------
create_cell("PIXEL");
// Pixel body on metal1.
add_rect("PIXEL", 1, 0, 0, 0, pixel_w, pixel_h);
// Centered via on via1.
let via_lo_x = (pixel_w - via) / 2;
let via_lo_y = (pixel_h - via) / 2;
add_rect("PIXEL", 2, 0, via_lo_x, via_lo_y, via_lo_x + via, via_lo_y + via);

// -------- tiled top cell --------
create_cell("SENSOR");
add_array("SENSOR", "PIXEL", 0, 0, columns, rows, pitch_x, pitch_y);
set_top_cells(["SENSOR"]);

// The array expands to columns * rows pixels, each with 2 shapes.
print("PIXEL shapes: " + shape_count("PIXEL"));
print("SENSOR arrays: " + array_count("SENSOR"));
print("flattened SENSOR shapes: " + flatten_count("SENSOR"));

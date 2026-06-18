// Run with: cargo run --example try_it
//
// This is a scratch file for trying your own JSON input and seeing the
// .kore output. Edit the `input` string below, or read from a file —
// see the commented-out section at the bottom for that.

use kore::{to_kore_from_str, KoreOptions};

fn main() {
    let input = r#"
    {
        "status": "ok",
        "page": 1,
        "hikes": [
            { "id": 1, "name": "Blue Lake Trail", "km": 7.5, "sunny": true },
            { "id": 2, "name": "Ridge Overlook",  "km": 9.2, "sunny": false }
        ]
    }
    "#;

    let opts = KoreOptions::new("hikes").infer_types(true);

    match to_kore_from_str(input, &opts) {
        Ok(result) => {
            println!("--- .kore output ---");
            println!("{}", result.kore);
            println!("--- metadata ---");
            println!("structure: {}", result.structure);
            println!("columns:   {:?}", result.columns);
            println!("row_count: {:?}", result.row_count);
        }
        Err(e) => eprintln!("Invalid JSON input: {e}"),
    }

    // To read from a real file instead of the inline string above:
    //
    // let input = std::fs::read_to_string("input.json").expect("can't read input.json");
    // let result = to_kore_from_str(&input, &opts).expect("invalid json");
    // std::fs::write("output.kore", &result.kore).expect("can't write output.kore");
}

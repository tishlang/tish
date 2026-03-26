# Data Processing Example

A real-world example demonstrating data manipulation in Tish: array sorting, modification, and nested loops for aggregating sales order data.

## Features Used

None (runs in secure mode). Uses built-in array methods and control flow only.

## What It Does

- **Sample data**: Orders with nested line items (id, customer, date, items)
- **Sorting**: Orders by date; line items by price
- **Modification**: Updates quantities via splice; adds new items via push
- **Nested loops**: Iterates orders and items to compute per-order totals and a grand total
- **Higher-order methods**: filter, map, reduce for data transformation
- Outputs a formatted sales report

## Local Development

Run without installing tish (from this directory; tish repo is parent):

```bash
# Run with interpreter
cargo run -p tishlang--manifest-path ../../Cargo.toml --release -- run src/main.tish

# Compile and run
cargo run -p tishlang--manifest-path ../../Cargo.toml --release -- compile src/main.tish -o data-processing
./data-processing
```

Or with tish installed: `tish run src/main.tish` and `tish compile src/main.tish -o data-processing`

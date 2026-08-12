# Example point-cloud data

`stadium-utm.laz` is a 693,895-point extract of the 2010 Autzen Stadium
LiDAR dataset. It is included as a compact real-world input for the
`renderer-demo` and `source-las` examples.

The data was provided by Aaron Reyna of Watershed Sciences, Inc. for libLAS
testing and is distributed by the [PDAL example-data repository][pdal-data]
under the [Creative Commons Attribution 4.0 International license][cc-by].

Run the interactive viewer from the repository root:

```bash
cargo run --release -p renderer-demo -- examples/data/stadium-utm.laz
```

The generated `.pidx` index files are ignored in this directory.

[pdal-data]: https://github.com/PDAL/data/tree/main/autzen
[cc-by]: https://creativecommons.org/licenses/by/4.0/

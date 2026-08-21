# Example point-cloud data

## `usgs-id-northcentral-11tnl550650.laz` (survey-quality recommendation)

This is a current USGS 3D Elevation Program tile from the 2023–2024 North
Central Idaho acquisition. The project used a 0.35 m nominal pulse spacing
(about 8.16 nominal pulses per square metre) and the USGS Lidar Base
Specification 2024 revision A. USGS 3DEP products are public domain.

- Source: [USGS direct download](https://rockyweb.usgs.gov/vdelivery/Datasets/Staged/Elevation/LPC/Projects/ID_NorthCentral_D22/ID_NCentral_5_D22/LAZ/USGS_LPC_ID_NorthCentral_D22_11TNL550650.laz)
- Project report: [USGS North Central Idaho report](https://prd-tnm.s3.amazonaws.com/StagedProducts/Elevation/metadata/ID_NorthCentral_D22/ID_NCentral_5_D22/reports/ID_NCentral_5_D22_Report.pdf)
- License: [USGS 3DEP public domain](https://data.usgs.gov/datacatalog/data/USGS%3Ab7e353d2-325f-4fc6-8d95-01254705638a)
- Size: 24,215,326 bytes
- SHA-256: `cbec161a788cc14529ad2aa474770e956916c193ac28acc543874e633cdb1909`
- Punctra inspection: 3,475,227 points, LAS 1.4 point format 6, 16
  supported attributes, 21 opaque Extra Bytes per point, metre/metre compound
  WKT, and a complete exact read

Download and verify it:

```bash
curl -fL \
  https://rockyweb.usgs.gov/vdelivery/Datasets/Staged/Elevation/LPC/Projects/ID_NorthCentral_D22/ID_NCentral_5_D22/LAZ/USGS_LPC_ID_NorthCentral_D22_11TNL550650.laz \
  -o examples/data/usgs-id-northcentral-11tnl550650.laz
echo "cbec161a788cc14529ad2aa474770e956916c193ac28acc543874e633cdb1909  examples/data/usgs-id-northcentral-11tnl550650.laz" \
  | shasum -a 256 -c -
```

Use this file for modern classified survey, scale, bounds, and opaque-WKT
coverage. It has no RGB. Punctra preserves its WKT, but the strict structured
CRS gate does not infer a GeoTIFF profile from WKT.

## `autzen-classified.laz` (RGB recommendation)

`autzen-classified.laz` is the higher-quality Punctra example. It contains the
same real-world Autzen Stadium survey as `autzen.laz`, reprocessed as LAS 1.4
point format 7 with explicit classification metadata and an embedded compound
horizontal/vertical WKT coordinate reference. It retains intensity, return
information, GPS time, and RGB.

- Source: [PDAL/data `autzen/autzen-classified.laz`](https://github.com/PDAL/data/blob/360327d2ae791b9d52c57b610a5a6b5c1b08c878/autzen/autzen-classified.laz)
- Source revision: `360327d2ae791b9d52c57b610a5a6b5c1b08c878`
- License: [CC BY 4.0](https://github.com/PDAL/data/blob/360327d2ae791b9d52c57b610a5a6b5c1b08c878/LICENSE)
- Size: 74,416,814 bytes
- SHA-256: `c8828215facfb4c9465c9a7db9d45cf2c0e9175a6eb5b21e89417978cc85cd51`
- Punctra inspection: 10,653,336 points, LAS 1.4 point format 7, 18
  supported attributes, WKT CRS, and a complete exact read

Download the exact source revision:

```bash
curl -fL \
  https://media.githubusercontent.com/media/PDAL/data/360327d2ae791b9d52c57b610a5a6b5c1b08c878/autzen/autzen-classified.laz \
  -o examples/data/autzen-classified.laz
echo "c8828215facfb4c9465c9a7db9d45cf2c0e9175a6eb5b21e89417978cc85cd51  examples/data/autzen-classified.laz" \
  | shasum -a 256 -c -
```

Its WKT uses US survey feet. Punctra can inspect and render it, but metre-only
terrain and workspace workflows must reject or explicitly handle that unit.

## `autzen.laz`

`autzen.laz` is a real-world airborne LiDAR cloud of the Autzen Stadium area
used by PDAL for testing and evaluation. It is large enough to exercise
Punctra's source, indexing, terrain, and renderer paths beyond the small
fixtures in this directory.

- Source: [PDAL/data `autzen/autzen.laz`](https://github.com/PDAL/data/blob/360327d2ae791b9d52c57b610a5a6b5c1b08c878/autzen/autzen.laz)
- Source revision: `360327d2ae791b9d52c57b610a5a6b5c1b08c878`
- License: [CC BY 4.0](https://github.com/PDAL/data/blob/360327d2ae791b9d52c57b610a5a6b5c1b08c878/LICENSE)
- Size: 56,350,988 bytes
- SHA-256: `944b947501156e45df1b3b9d25bc1dc04ff5ef377e7e169576ba59231c2896ba`
- Punctra inspection: 10,653,336 points, LAS 1.2 point format 3 attributes,
  including intensity, return information, classification, GPS time, and RGB

Download the exact source revision:

```bash
curl -fL \
  https://media.githubusercontent.com/media/PDAL/data/360327d2ae791b9d52c57b610a5a6b5c1b08c878/autzen/autzen.laz \
  -o examples/data/autzen.laz
echo "944b947501156e45df1b3b9d25bc1dc04ff5ef377e7e169576ba59231c2896ba  examples/data/autzen.laz" \
  | shasum -a 256 -c -
```

Inspect it with Punctra:

```bash
cargo run --release -p source-las --example inspect -- \
  examples/data/autzen.laz
```

The file's projection metadata does not resolve to Punctra's strict supported
projected-reference profile, so Punctra correctly reports its coordinate
reference as explicitly unknown. Prefer `autzen-classified.laz` for current
format, classification, and CRS testing.

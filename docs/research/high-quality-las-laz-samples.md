# Higher-quality LAS/LAZ sample candidates

Research date: 2026-08-21. Sources below are first-party data catalogs,
project metadata, specifications, repositories, and the files themselves.
The selected candidates were subsequently downloaded and subjected to complete
Punctra exact reads. The selected binaries are present locally under
`examples/data`; whether to commit these large artifacts remains a repository
policy decision.

## Recommendation

Use two complementary files:

- **USGS `11TNL550650`** for the highest acquisition and survey quality that is
  currently verified by Punctra: a 2023–2024 public-domain survey, metric CRS,
  LAS 1.4 point format 6, and modern classification.
- **`autzen-classified.laz`** when RGB or a larger renderer cloud matters more.
  It is LAS 1.4 point format 7 and has also passed a complete exact read.

### RGB recommendation

`autzen-classified.laz` provides:

- 74,416,814 bytes and 10,653,336 points;
- LAS 1.4, regular LAZ point format 7;
- intensity, return information, classification, GPS time, and RGB;
- embedded compound horizontal/vertical WKT;
- commit-pinned CC BY 4.0 source and a verified SHA-256;
- Punctra reports all 18 supported attributes and completes an exact full read.

Direct download:
<https://media.githubusercontent.com/media/PDAL/data/360327d2ae791b9d52c57b610a5a6b5c1b08c878/autzen/autzen-classified.laz>

SHA-256:
`c8828215facfb4c9465c9a7db9d45cf2c0e9175a6eb5b21e89417978cc85cd51`

This improves format/metadata/CRS coverage and retains RGB, but it is a
reprocessing of the same acquisition as the old Autzen sample.

### Validated practical USGS tile

The smaller tile `USGS_LPC_ID_NorthCentral_D22_11TNL550650.laz` from the same
project was selected as the practical survey-quality example and passed a
complete exact Punctra read.

- Direct download:
  <https://rockyweb.usgs.gov/vdelivery/Datasets/Staged/Elevation/LPC/Projects/ID_NorthCentral_D22/ID_NCentral_5_D22/LAZ/USGS_LPC_ID_NorthCentral_D22_11TNL550650.laz>
- Local filename: `examples/data/usgs-id-northcentral-11tnl550650.laz`
- 24,215,326 bytes and 3,475,227 points
- LAS 1.4, regular LAZ point format 6, including a 21-byte opaque Extra Bytes
  slab per point
- compound NAD83(2011) / UTM zone 11N + NAVD88 / GEOID18 WKT, with horizontal
  and vertical units in metres
- SHA-256:
  `cbec161a788cc14529ad2aa474770e956916c193ac28acc543874e633cdb1909`
- Punctra exposed 16 supported attributes and completed the full exact read

This is the immediate survey-quality recommendation. It shares the larger
tile's acquisition specification and provenance while remaining smaller than
the original Autzen example. Use `autzen-classified.laz` instead where RGB is
required.

### Larger USGS stress candidate

For a genuinely newer, denser acquisition and a larger stress corpus, the best
next candidate to validate is
`USGS_LPC_ID_NorthCentral_D22_11TNL560350.laz`:

- Direct download (USGS):
  <https://rockyweb.usgs.gov/vdelivery/Datasets/Staged/Elevation/LPC/Projects/ID_NorthCentral_D22/ID_NCentral_5_D22/LAZ/USGS_LPC_ID_NorthCentral_D22_11TNL560350.laz>
- Official directory listing:
  <https://rockyweb.usgs.gov/vdelivery/Datasets/Staged/Elevation/LPC/Projects/ID_NorthCentral_D22/ID_NCentral_5_D22/LAZ/>
- Project report:
  <https://prd-tnm.s3.amazonaws.com/StagedProducts/Elevation/metadata/ID_NorthCentral_D22/ID_NCentral_5_D22/reports/ID_NCentral_5_D22_Report.pdf>
- NOAA's first-party catalog record for the USGS project:
  <https://www.fisheries.noaa.gov/inport/item/78959>

The official file and its LAS header report:

- 192,273,312 bytes;
- 17,571,822 points;
- LAS 1.4, compressed point-data record format 6, 51-byte records;
- intensity, return information, classification, scan channel/angle, point
  source ID, user data, and GPS time from point format 6;
- a 21-byte trailing Extra Bytes slab per record (Punctra preserves this as
  uninterpreted fixed bytes rather than publishing invented dimension
  meanings);
- embedded `LASF_Projection:2112` compound WKT for NAD83(2011) / UTM zone 11N
  and NAVD88 / GEOID18, with both horizontal and vertical units in metres;
- no RGB, because point format 6 has no color channels.

The Idaho tile is a material upgrade over the current Autzen file for acquisition
provenance, modern LAS semantics, point count, embedded CRS, classification,
and stress coverage. The project was acquired in 2023–2024 at a nominal pulse
spacing of 0.35 m (about 8.16 nominal pulses/m²), follows the USGS Lidar Base
Specification 2024 revision A, and produced a 0.5 m ground-derived DEM. The
NOAA catalog confirms class 2 ground use and the metric NAD83(2011) / NAVD88
reference. USGS requires modern point deliverables to use LAS 1.4-R15 and
formats 6–10, with standardized classification and a single valid compliant
WKT CRS entry: <https://www.usgs.gov/ngp-standards-and-specifications/lidar-base-specification-data-processing-and-handling-requirements>.

It is also the best not-yet-validated regular-LAZ fit among the high-quality candidates below.
Punctra supports regular LAZ formats 0–8, including format 6, but explicitly
does not claim COPC support. Its adapter will expose this file's WKT as opaque
WKT; consequently, Source inspection, indexing, and rendering are appropriate,
but the strict `terrain-demo` metre/metre structured-profile gate will still
reject it. That gate accepts only a complete structured GeoTIFF profile, not
opaque WKT. See `crates/source-las/src/lib.rs` and `CONTRIBUTING.md`.

USGS publishes 3DEP products as public domain:
<https://data.usgs.gov/datacatalog/data/USGS%3Ab7e353d2-325f-4fc6-8d95-01254705638a>.
No authoritative SHA-256 was found for the individual tile. The server ETag
`"68bf8d27-b75dba0"` is not documented as a content checksum and must not be
treated as one. After downloading, record a local SHA-256 and byte count:

```sh
curl -fL \
  'https://rockyweb.usgs.gov/vdelivery/Datasets/Staged/Elevation/LPC/Projects/ID_NorthCentral_D22/ID_NCentral_5_D22/LAZ/USGS_LPC_ID_NorthCentral_D22_11TNL560350.laz' \
  -o examples/data/usgs-id-northcentral-11tnl560350.laz
shasum -a 256 examples/data/usgs-id-northcentral-11tnl560350.laz
wc -c examples/data/usgs-id-northcentral-11tnl560350.laz
```

## Comparison

| Candidate | Size / points | Format and attributes | CRS and classification | Acquisition quality / license | Punctra fit |
| --- | ---: | --- | --- | --- | --- |
| Current `autzen.laz` | 56,350,988 B / 10,653,336 | LAS 1.2, PF3; intensity, returns, GPS time, RGB | Classified, but its GeoTIFF metadata does not pass Punctra's strict CRS profile | Older real-world sample; PDAL data is CC BY 4.0 | Supported regular LAZ, but CRS remains unknown |
| **`autzen-classified.laz` (immediate recommendation)** | **74,416,814 B / 10,653,336** | **LAS 1.4, PF7; intensity, returns, GPS time, RGB** | **Classified; compound WKT** | Same acquisition reprocessed; CC BY 4.0 | **Verified complete exact Punctra read; best immediate renderer/format fixture** |
| **USGS North Central Idaho `11TNL550650` (survey recommendation)** | **24,215,326 B / 3,475,227** | **LAS 1.4, PF6; 21-byte extra slab; no RGB** | **Classified; compound metre/metre WKT** | **2023–2024; 0.35 m NPS; USGS 2024 rev. A; public domain** | **Verified complete exact Punctra read; best practical modern survey fixture** |
| **USGS North Central Idaho `11TNL560350` (next candidate)** | **192,273,312 B / 17,571,822** | **LAS 1.4, PF6; 21-byte extra slab; no RGB** | **Classified; compound metre/metre WKT** | **2023–2024; 0.35 m NPS; USGS 2024 rev. A; public domain** | **Best new-acquisition regular-LAZ quality/stress candidate; full Punctra read still needs validation; WKT remains opaque to strict terrain workflow** |
| USGS Wisconsin Dodge County `801634` | 60,046,350 B / 12,542,034 | LAS 1.4, PF6; no RGB | Classified; compound WKT, but US survey feet | 2017 acquisition; USGS public domain | Best near-current-size regular-LAZ alternative; not metric and no RGB |
| PDAL `autzen-2023.copc.laz` | 184,509,941 B / 21,233,219 | LAS 1.4, PF7; RGB | Very rich ground, vegetation, building, water, vehicle, bridge, utility, and structure classes; compound CRS | 2023; 0.22 m NPS; measured NVA RMSEz 0.046 m; CC BY 4.0 | Visually strongest, but **incompatible now**: Punctra's exact read fails with bounds corruption |
| NOAA Entiat Upper WA COPC tile | 332,538,914 B / 61,503,559 | LAS 1.4, PF6; no RGB | Classified; compound CRS | Authoritative NOAA-hosted forestry/mountain data; CC BY 1.0 in STAC | Excellent stress corpus, but COPC is unsupported |
| swisstopo swissSURFACE3D current tiles | about 200 MB/tile; several million/km² | LAS 1.4 COPC; no RGB stated | Classified, LV95/LN02 | Minimum 15 and mean 25–40 points/m²; ±20 cm XY and ±10 cm Z (1 sigma); sample is test-use only | Exceptional survey quality, but COPC plus restrictive sample terms make it unsuitable for repository inclusion |

### Smaller regular-LAZ alternative

The Wisconsin tile is useful when 192 MB is too heavy:

<https://rockyweb.usgs.gov/vdelivery/Datasets/Staged/Elevation/LPC/Projects/USGS_LPC_WI_DodgeCo_2017_LAS_2019/laz/USGS_LPC_WI_DodgeCo_2017_801634_LAS_2019.laz>

Small HTTP range inspection confirmed LAS 1.4, compressed format 6, 12,542,034
points, and a `LASF_Projection:2112` compound WKT for NAD83(2011) / WISCRS
Dodge and Jefferson plus NAVD88 / GEOID12B in US survey feet. Its official
directory supplies the 60,046,350-byte size:
<https://rockyweb.usgs.gov/vdelivery/Datasets/Staged/Elevation/LPC/Projects/USGS_LPC_WI_DodgeCo_2017_LAS_2019/laz/>.
No authoritative per-file SHA-256 was found.

### Visually strongest candidate, currently incompatible

PDAL's 2023 Autzen file is the clear choice if Punctra later adds COPC support
or if the file is converted into a regular, lossless LAZ outside the repo:

- Commit-pinned binary:
  <https://media.githubusercontent.com/media/PDAL/data/ce0024257c573526389c4db9ab26e82739b8aaa9/autzen/2023/autzen-2023.copc.laz>
- SHA-256 (the repository's Git LFS object ID):
  `8fd9aed76549c52d5b9680a4ef27ecf4a8ff765f3c8b1b44864926551bb2920e`
- Acquisition and classification metadata:
  <https://raw.githubusercontent.com/PDAL/data/ce0024257c573526389c4db9ab26e82739b8aaa9/autzen/2023/OLC_Willamette_Valley_Classified_LAS_Metadata.xml>
- Reproducible PDAL processing recipe:
  <https://raw.githubusercontent.com/PDAL/data/ce0024257c573526389c4db9ab26e82739b8aaa9/autzen/2023/process.sh>
- PDAL data license (CC BY 4.0):
  <https://raw.githubusercontent.com/PDAL/data/ce0024257c573526389c4db9ab26e82739b8aaa9/LICENSE>

The file is LAS 1.4 / point format 7 with RGB, 21,233,219 points, 0.22 m
nominal pulse spacing, and measured non-vegetated vertical RMSE of 0.046 m.
It is a particularly good future renderer corpus because it covers the same
place as the present sample while roughly doubling the point count and adding
far richer classifications. However, this is not merely a theoretical support
caveat: a local Punctra full-read attempt failed with
`Source is corrupt: decoded Point bounds extend beyond the LAS public header`.
It must not be adopted as a current fixture without COPC adapter work and a
passing exact-read acceptance test.

NOAA's larger Entiat COPC stress option is documented by its first-party STAC
item and collection metadata:

- <https://noaa-nos-coastal-lidar-pds.s3.amazonaws.com/laz/geoid18/9580/stac/20171003_OKAFCW_552000_1551000.copc.json>
- <https://noaa-nos-coastal-lidar-pds.s3.amazonaws.com/laz/geoid18/9580/metadata_wa2017_entiat.xml>

swisstopo's official product page documents swissSURFACE3D density, accuracy,
format, tile size, and sample-use restriction:
<https://www.swisstopo.admin.ch/en/height-model-swisssurface3d>.

## Decision

Adopt **USGS North Central Idaho `11TNL550650`** as the immediate modern-survey
example: it is recent, metric, public domain, checksummed, and passed Punctra's
exact full read. Adopt **`autzen-classified.laz`** alongside it when RGB and a
larger renderer cloud matter more than acquisition recency. Validate USGS North
Central Idaho `11TNL560350` next only when a roughly 192 MB stress fixture is
acceptable. Keep large corpora out of
normal unit-test and source-control paths unless Git LFS or an explicit
external-corpus policy is adopted. Use the 60 MB Wisconsin tile for a lighter
smoke corpus. Do not adopt a COPC file until Punctra explicitly supports COPC
layout and verifies it in its adapter contract.

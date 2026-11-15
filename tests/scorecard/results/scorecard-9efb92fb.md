# Scorecard of 9efb92fb

Facts that hold, of the facts checked, per set, half and class. `dev` files may be named in a task; `held` files are held out.

How to read the sets:

- **drift check (truth from Rezzer's own tagged read)**: the truth of these sets was made from Rezzer's read of the files with their tags. They hold by construction on the day the truth is made. A lower number later shows that the tagged read moved. They are no evidence that a read is right.
- **lower bound**: the same files read with their tags taken out, against that truth. Text extraction is shared with the truth, and tag conventions that the layout does not show count as failures. The numbers are a lower bound on structure, not a proof of right values.
- Facts from invoice XML check that the data is present and that an item line is one row. They do not check which label an amount stands with.
- `all` leaves out the `link` class: links come from annotations, not from layout.
- A set with assertion files that have no read is not whole: their facts are in no count.

| set | what it is | class | dev | held |
|---|---|---|---:|---:|
| corpus | drift check (truth from Rezzer's own tagged read) | table | 4 / 4 (100.0 %) | 494 / 494 (100.0 %) |
| corpus | drift check (truth from Rezzer's own tagged read) | order | 143 / 143 (100.0 %) | 222 / 222 (100.0 %) |
| corpus | drift check (truth from Rezzer's own tagged read) | heading | 15 / 15 (100.0 %) | 202 / 202 (100.0 %) |
| corpus | drift check (truth from Rezzer's own tagged read) | text | 124 / 124 (100.0 %) | 216 / 216 (100.0 %) |
| corpus | drift check (truth from Rezzer's own tagged read) | link | 1 / 1 (100.0 %) | 196 / 196 (100.0 %) |
| corpus | drift check (truth from Rezzer's own tagged read) | **all but link** | 286 / 286 (100.0 %) | 1134 / 1134 (100.0 %) |
| corpus-untagged | lower bound | table | 0 / 4 (0.0 %) | 144 / 494 (29.1 %) |
| corpus-untagged | lower bound | order | 141 / 143 (98.6 %) | 218 / 222 (98.2 %) |
| corpus-untagged | lower bound | heading | 6 / 15 (40.0 %) | 25 / 202 (12.4 %) |
| corpus-untagged | lower bound | text | 116 / 124 (93.5 %) | 199 / 216 (92.1 %) |
| corpus-untagged | lower bound | link | 1 / 1 (100.0 %) | 196 / 196 (100.0 %) |
| corpus-untagged | lower bound | **all but link** | 263 / 286 (92.0 %) | 586 / 1134 (51.7 %) |
| forms | drift check (truth from Rezzer's own tagged read) | table | 211 / 213 (99.1 %) | 132 / 134 (98.5 %) |
| forms | drift check (truth from Rezzer's own tagged read) | order | 388 / 388 (100.0 %) | 51 / 51 (100.0 %) |
| forms | drift check (truth from Rezzer's own tagged read) | heading | 225 / 225 (100.0 %) | 3 / 3 (100.0 %) |
| forms | drift check (truth from Rezzer's own tagged read) | text | 387 / 387 (100.0 %) | 137 / 137 (100.0 %) |
| forms | drift check (truth from Rezzer's own tagged read) | link | 39 / 39 (100.0 %) | 10 / 10 (100.0 %) |
| forms | drift check (truth from Rezzer's own tagged read) | **all but link** | 1211 / 1213 (99.8 %) | 323 / 325 (99.4 %) |
| forms-untagged | lower bound | table | 5 / 210 (2.4 %) | 43 / 131 (32.8 %) |
| forms-untagged | lower bound | order | 385 / 388 (99.2 %) | 51 / 51 (100.0 %) |
| forms-untagged | lower bound | heading | 16 / 225 (7.1 %) | 3 / 3 (100.0 %) |
| forms-untagged | lower bound | text | 364 / 380 (95.8 %) | 123 / 126 (97.6 %) |
| forms-untagged | lower bound | link | 39 / 39 (100.0 %) | 10 / 10 (100.0 %) |
| forms-untagged | lower bound | **all but link** | 770 / 1203 (64.0 %) | 220 / 311 (70.7 %) |
| generated |  | table | 4840 / 5324 (90.9 %) | 6407 / 6422 (99.8 %) |
| generated |  | pair | 1794 / 1958 (91.6 %) | 5980 / 6124 (97.6 %) |
| generated |  | order | 252 / 285 (88.4 %) | - |
| generated |  | heading | 33 / 33 (100.0 %) | 183 / 183 (100.0 %) |
| generated |  | text | 132 / 171 (77.2 %) | 171 / 210 (81.4 %) |
| generated |  | **all but link** | 7051 / 7771 (90.7 %) | 12741 / 12939 (98.5 %) |

### Assertion files with no read

| set | dev | held |
|---|---:|---:|
| corpus | 0 | 0 |
| corpus-untagged | 0 | 0 |
| forms | 0 | 0 |
| forms-untagged | 0 | 0 |
| generated | 0 | 0 |

## Regions

Regions with a type, of all regions, and the regions kept as laid out (`spatial`). Facts on typed regions: the facts whose text stands in a region with a type, and how many of them hold.

| set | half | typed regions | spatial | facts on typed regions | facts on spatial regions | facts with no region | facts whose text is not found |
|---|---|---:|---:|---:|---:|---:|---:|
| corpus | dev | 884 / 957 (92.4 %) | 73 / 957 (7.6 %) | 287 / 287 (100.0 %) | - | - | - |
| corpus | held | 9275 / 9803 (94.6 %) | 528 / 9803 (5.4 %) | 1107 / 1107 (100.0 %) | 2 / 2 (100.0 %) | 221 / 221 (100.0 %) | - |
| corpus-untagged | dev | 264 / 318 (83.0 %) | 54 / 318 (17.0 %) | 80 / 87 (92.0 %) | 184 / 191 (96.3 %) | - | 0 / 9 (0.0 %) |
| corpus-untagged | held | 7793 / 9586 (81.3 %) | 1793 / 9586 (18.7 %) | 635 / 786 (80.8 %) | 111 / 289 (38.4 %) | 36 / 234 (15.4 %) | 0 / 21 (0.0 %) |
| forms | dev | 4348 / 4580 (94.9 %) | 232 / 4580 (5.1 %) | 1235 / 1236 (99.9 %) | 14 / 15 (93.3 %) | 1 / 1 (100.0 %) | - |
| forms | held | 1577 / 1745 (90.4 %) | 168 / 1745 (9.6 %) | 325 / 326 (99.7 %) | 4 / 5 (80.0 %) | 4 / 4 (100.0 %) | - |
| forms-untagged | dev | 3230 / 3583 (90.1 %) | 353 / 3583 (9.9 %) | 598 / 815 (73.4 %) | 117 / 148 (79.1 %) | 93 / 258 (36.0 %) | 1 / 21 (4.8 %) |
| forms-untagged | held | 720 / 799 (90.1 %) | 79 / 799 (9.9 %) | 189 / 220 (85.9 %) | 14 / 15 (93.3 %) | 26 / 82 (31.7 %) | 1 / 4 (25.0 %) |
| generated | dev | 10399 / 10953 (94.9 %) | 554 / 10953 (5.1 %) | 6976 / 7538 (92.5 %) | 3 / 3 (100.0 %) | 72 / 105 (68.6 %) | 0 / 125 (0.0 %) |
| generated | held | 13374 / 14101 (94.8 %) | 727 / 14101 (5.2 %) | 12741 / 12872 (99.0 %) | - | 0 / 28 (0.0 %) | 0 / 39 (0.0 %) |

### Per kind

| set | half | kind | regions | facts that hold |
|---|---|---|---:|---:|
| corpus | dev | header_band | 30 | - |
| corpus | dev | footer_band | 16 | - |
| corpus | dev | title_block | 2 | - |
| corpus | dev | heading | 29 | 22 / 22 (100.0 %) |
| corpus | dev | paragraph | 739 | 253 / 253 (100.0 %) |
| corpus | dev | list | 23 | 8 / 8 (100.0 %) |
| corpus | dev | table | 3 | 4 / 4 (100.0 %) |
| corpus | dev | label_value | 14 | - |
| corpus | dev | figure | 27 | - |
| corpus | dev | footnote_legend | 1 | - |
| corpus | dev | spatial | 73 | - |
| corpus | held | header_band | 1058 | - |
| corpus | held | footer_band | 467 | 3 / 3 (100.0 %) |
| corpus | held | title_block | 3 | - |
| corpus | held | heading | 399 | 16 / 16 (100.0 %) |
| corpus | held | paragraph | 4736 | 393 / 393 (100.0 %) |
| corpus | held | list | 2007 | 164 / 164 (100.0 %) |
| corpus | held | table | 104 | 531 / 531 (100.0 %) |
| corpus | held | figure | 501 | - |
| corpus | held | spatial | 528 | 2 / 2 (100.0 %) |
| corpus | held | (no region) | 0 | 221 / 221 (100.0 %) |
| corpus-untagged | dev | title_block | 1 | 2 / 2 (100.0 %) |
| corpus-untagged | dev | heading | 9 | 9 / 10 (90.0 %) |
| corpus-untagged | dev | paragraph | 249 | 66 / 72 (91.7 %) |
| corpus-untagged | dev | list | 4 | 3 / 3 (100.0 %) |
| corpus-untagged | dev | table | 1 | - |
| corpus-untagged | dev | spatial | 54 | 184 / 191 (96.3 %) |
| corpus-untagged | dev | (not found) | 0 | 0 / 9 (0.0 %) |
| corpus-untagged | held | header_band | 1032 | - |
| corpus-untagged | held | footer_band | 472 | 5 / 5 (100.0 %) |
| corpus-untagged | held | title_block | 2 | 2 / 2 (100.0 %) |
| corpus-untagged | held | heading | 50 | 2 / 2 (100.0 %) |
| corpus-untagged | held | paragraph | 5293 | 433 / 496 (87.3 %) |
| corpus-untagged | held | list | 207 | 14 / 15 (93.3 %) |
| corpus-untagged | held | table | 18 | 21 / 31 (67.7 %) |
| corpus-untagged | held | label_value | 189 | 4 / 7 (57.1 %) |
| corpus-untagged | held | figure | 500 | 154 / 228 (67.5 %) |
| corpus-untagged | held | footnote_legend | 30 | - |
| corpus-untagged | held | spatial | 1793 | 111 / 289 (38.4 %) |
| corpus-untagged | held | (no region) | 0 | 36 / 234 (15.4 %) |
| corpus-untagged | held | (not found) | 0 | 0 / 21 (0.0 %) |
| forms | dev | header_band | 62 | - |
| forms | dev | footer_band | 244 | - |
| forms | dev | title_block | 6 | 2 / 2 (100.0 %) |
| forms | dev | heading | 1317 | 302 / 302 (100.0 %) |
| forms | dev | paragraph | 1618 | 510 / 511 (99.8 %) |
| forms | dev | list | 807 | 227 / 227 (100.0 %) |
| forms | dev | table | 81 | 192 / 192 (100.0 %) |
| forms | dev | label_value | 83 | 1 / 1 (100.0 %) |
| forms | dev | totals | 10 | - |
| forms | dev | figure | 109 | 1 / 1 (100.0 %) |
| forms | dev | code | 11 | - |
| forms | dev | spatial | 232 | 14 / 15 (93.3 %) |
| forms | dev | (no region) | 0 | 1 / 1 (100.0 %) |
| forms | held | footer_band | 41 | - |
| forms | held | title_block | 7 | 2 / 2 (100.0 %) |
| forms | held | heading | 45 | 2 / 2 (100.0 %) |
| forms | held | paragraph | 1311 | 181 / 182 (99.5 %) |
| forms | held | list | 29 | 8 / 8 (100.0 %) |
| forms | held | table | 58 | 129 / 129 (100.0 %) |
| forms | held | label_value | 72 | 3 / 3 (100.0 %) |
| forms | held | figure | 13 | - |
| forms | held | code | 1 | - |
| forms | held | spatial | 168 | 4 / 5 (80.0 %) |
| forms | held | (no region) | 0 | 4 / 4 (100.0 %) |
| forms-untagged | dev | header_band | 115 | - |
| forms-untagged | dev | footer_band | 203 | - |
| forms-untagged | dev | title_block | 4 | 5 / 6 (83.3 %) |
| forms-untagged | dev | heading | 116 | 21 / 32 (65.6 %) |
| forms-untagged | dev | paragraph | 1778 | 268 / 432 (62.0 %) |
| forms-untagged | dev | list | 720 | 242 / 244 (99.2 %) |
| forms-untagged | dev | table | 63 | 58 / 94 (61.7 %) |
| forms-untagged | dev | label_value | 127 | - |
| forms-untagged | dev | totals | 1 | - |
| forms-untagged | dev | figure | 92 | 4 / 7 (57.1 %) |
| forms-untagged | dev | code | 11 | - |
| forms-untagged | dev | spatial | 353 | 117 / 148 (79.1 %) |
| forms-untagged | dev | (no region) | 0 | 93 / 258 (36.0 %) |
| forms-untagged | dev | (not found) | 0 | 1 / 21 (4.8 %) |
| forms-untagged | held | title_block | 4 | 3 / 3 (100.0 %) |
| forms-untagged | held | heading | 21 | 7 / 7 (100.0 %) |
| forms-untagged | held | paragraph | 676 | 142 / 169 (84.0 %) |
| forms-untagged | held | list | 2 | 8 / 8 (100.0 %) |
| forms-untagged | held | table | 6 | 27 / 31 (87.1 %) |
| forms-untagged | held | label_value | 7 | 2 / 2 (100.0 %) |
| forms-untagged | held | totals | 1 | - |
| forms-untagged | held | figure | 3 | - |
| forms-untagged | held | spatial | 79 | 14 / 15 (93.3 %) |
| forms-untagged | held | (no region) | 0 | 26 / 82 (31.7 %) |
| forms-untagged | held | (not found) | 0 | 1 / 4 (25.0 %) |
| generated | dev | footer_band | 136 | - |
| generated | dev | title_block | 570 | 33 / 33 (100.0 %) |
| generated | dev | heading | 184 | - |
| generated | dev | paragraph | 5448 | 222 / 330 (67.3 %) |
| generated | dev | table | 1600 | 3428 / 3817 (89.8 %) |
| generated | dev | label_value | 777 | 2046 / 2111 (96.9 %) |
| generated | dev | totals | 490 | 1247 / 1247 (100.0 %) |
| generated | dev | figure | 396 | - |
| generated | dev | code | 764 | - |
| generated | dev | footnote_legend | 34 | - |
| generated | dev | spatial | 554 | 3 / 3 (100.0 %) |
| generated | dev | (no region) | 0 | 72 / 105 (68.6 %) |
| generated | dev | (not found) | 0 | 0 / 125 (0.0 %) |
| generated | held | footer_band | 270 | - |
| generated | held | title_block | 688 | 66 / 66 (100.0 %) |
| generated | held | heading | 347 | 117 / 117 (100.0 %) |
| generated | held | paragraph | 6424 | 204 / 208 (98.1 %) |
| generated | held | table | 2032 | 5034 / 5049 (99.7 %) |
| generated | held | label_value | 1719 | 6227 / 6339 (98.2 %) |
| generated | held | totals | 458 | 1093 / 1093 (100.0 %) |
| generated | held | figure | 586 | - |
| generated | held | code | 782 | - |
| generated | held | footnote_legend | 68 | - |
| generated | held | spatial | 727 | - |
| generated | held | (no region) | 0 | 0 / 28 (0.0 %) |
| generated | held | (not found) | 0 | 0 / 39 (0.0 %) |

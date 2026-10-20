# Test corpus

Public files with licenses that allow redistribution. Each file is data: it is only read, never executed. "Derived" means the file was changed from a public source (transcoded, cut); it keeps the license of its source. License texts and attributions are in `LICENSES.md`. Large files that are not in the repo are listed in `remote.tsv` and fetched with `scripts/fetch-corpus.sh`.

Every file has a `<file>.facts.json` next to it and golden snapshots in `snapshots/`.

| File | Source | License | SHA-256 |
|---|---|---|---|
| web-text/csv-broken-derived-banklist.csv | derived from the first 12 lines of banklist.csv | BSD-3-Clause / public domain | aa22bb95031ea5ff6165378d0e74c4387b5746722c63571dcb6d3d95816ec2a6 |
| web-text/csv-datahub-co2-fossil-global.csv | https://github.com/datasets/co2-fossil-global/blob/main/data/global.csv | ODC-PDDL-1.0 | 289acefc1c96f74e23b852c1b591c6444b9c85d5663b3cffd912222f4999b546 |
| web-text/csv-latin1-noheader-derived-geonames.csv | derived from admin1CodesASCII.txt (rows AT, CH, DE, ES, FR, comma separated, Latin-1) | CC BY 4.0 (GeoNames) | c3b48ea33e7ddc1c46b94bd1c4030f76e285a6cc9530a5b0603294912175b37e |
| web-text/csv-pandas-fdic-banklist.csv | https://github.com/pandas-dev/pandas/blob/main/pandas/tests/io/data/csv/banklist.csv | BSD-3-Clause (pandas); data from the US FDIC, public domain | b33f2443e72c52ed02c8f9bbd4969d69590373f75c1b0f3b07e3693127c48423 |
| web-text/csv-semicolon-latin1-dwd-stations.txt | https://opendata.dwd.de/climate_environment/CDC/observations_germany/climate/multi_annual/mean_91-20/Temperatur_1991-2020_Stationsliste.txt | CC BY 4.0 (Deutscher Wetterdienst) | 6d2b0ee5d3c33f27cb36e773b0fa35bfc3468cbac4306d89010f7b3f5bd8387b |
| web-text/csv-spectrum-comma_in_quotes.csv | https://github.com/max-mapper/csv-spectrum/tree/master/csvs | BSD-2-Clause (package.json; not MIT) | f7654bf8e69586c8fe7f7f89392d2ed551be0e468e7101187bb482357d9a7815 |
| web-text/csv-spectrum-empty.csv | https://github.com/max-mapper/csv-spectrum/tree/master/csvs | BSD-2-Clause (package.json; not MIT) | f868c0f521228d5afc2c21eac9ad98729d2db969b98126105be844856d9b952a |
| web-text/csv-spectrum-escaped_quotes.csv | https://github.com/max-mapper/csv-spectrum/tree/master/csvs | BSD-2-Clause (package.json; not MIT) | a0d378e3045aefd50a6eacb40d8ab488a2f3cadcbb79f1cfd727eba95bbb0ca3 |
| web-text/csv-spectrum-json.csv | https://github.com/max-mapper/csv-spectrum/tree/master/csvs | BSD-2-Clause (package.json; not MIT) | 27bebe48687aa0cc5858692c49e390f129b79063cd032caf8e329ca8b5ded355 |
| web-text/csv-spectrum-newlines_crlf.csv | https://github.com/max-mapper/csv-spectrum/tree/master/csvs | BSD-2-Clause (package.json; not MIT) | 7d05c17ec14367b2cf0dc4777861b575b6ccc2a1d5145194f2dba4a1b8d9056f |
| web-text/csv-spectrum-quotes_and_newlines.csv | https://github.com/max-mapper/csv-spectrum/tree/master/csvs | BSD-2-Clause (package.json; not MIT) | f4d99e9a37ab4e7384c494f75a1f252e5e13efed0b7de0dc00050f3517930c2f |
| web-text/md-toml-frontmatter-rust-blog-traits.md | https://github.com/rust-lang/blog.rust-lang.org/blob/main/content/traits.md | MIT OR Apache-2.0 | 3c8c21ca977b691b0f0292ed63778361377d3161703164b3aadfc5e03b07e53d |
| web-text/md-wikilinks-foam-docs.md | https://github.com/foambubble/foam/blob/main/docs/user/features/wikilinks.md | MIT (Foam) | d36b6cbab90d8a9ca7c581f3fdda417d8386310d0f0b22f401669e185be07088 |
| web-text/md-yaml-frontmatter-github-docs-about-readmes.md | https://github.com/github/docs/blob/main/content/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/about-readmes.md | CC BY 4.0 (GitHub docs content) | 5a3dfd2042720a4cc1f0ff77660cf21714eb98b127932d7a44976663f9509d3b |
| web-text/text-cp1252-derived-gutenberg-alice.txt | derived: https://www.gutenberg.org/cache/epub/11/pg11.txt transcoded from UTF-8 to Windows-1252 | Public domain in the USA | 233b828e6dd75253e67925d33bb427209250ba40eedce0d3b25262b89ff74047 |
| web-text/text-latin1-gutenberg-verwandlung.txt | https://www.gutenberg.org/files/22367/22367-8.txt | Public domain in the USA | fe91c871104780ce5a8fe3e12e79b757f6fb5ffa28ed83d6186e618fb3bf344a |
| web-text/text-shiftjis-aozora-botchan.txt | https://www.aozora.gr.jp/cards/000148/files/752_ruby_2438.zip (bocchan.txt) | Public domain (Aozora Bunko) | 8b1087162da44dbf54705c15f5ba62c7c07db86bb6f38caacd4f5beb62e4618b |
| web-text/text-utf8-gutenberg-faust.txt | https://www.gutenberg.org/cache/epub/2229/pg2229.txt | Public domain in the USA (Project Gutenberg) | fed2a58edd6910ee96d27ada614d34bcc41f03750d86541c41246bb74c564ff7 |
| web-text/tsv-noheader-geonames-admin1.txt | https://download.geonames.org/export/dump/admin1CodesASCII.txt | CC BY 4.0 (GeoNames) | b577baa3476bbe94f0454545e517e1835401d79c5b1105c8e02cf1d4a505e8ec |

# Licenses of the corpus files

Each file keeps the license of its source; `MANIFEST.md` gives the license per file. This file
holds the attributions and the license texts that come with a redistribution. License texts
were fetched on 2026-10-20 from the URLs named below (branch heads, no pinned commit).

## Attribution licenses (CC BY 4.0)

License text: https://creativecommons.org/licenses/by/4.0/legalcode . The files were not
changed, except where `MANIFEST.md` says "derived".

- `csv-semicolon-latin1-dwd-stations.txt`: Source: Deutscher Wetterdienst (DWD), Climate Data
  Center, https://opendata.dwd.de/climate_environment/CDC/observations_germany/climate/multi_annual/mean_91-20/Temperatur_1991-2020_Stationsliste.txt .
  The DWD terms for the CDC open data area (https://opendata.dwd.de/climate_environment/CDC/Nutzungsbedingungen_German.txt , "Stand: Mai 2024")
  say: the conditions of the license Creative Commons BY 4.0 apply, details at https://www.dwd.de/copyright .
  Checked on 2026-10-20: the file is inside that area.
- `tsv-noheader-geonames-admin1.txt` and `csv-latin1-noheader-derived-geonames.csv` (derived:
  five countries, comma separated, Latin-1): Source: GeoNames, https://www.geonames.org/ ,
  file https://download.geonames.org/export/dump/admin1CodesASCII.txt . GeoNames data is
  licensed under CC BY 4.0 (https://www.geonames.org/export/).
- `md-yaml-frontmatter-github-docs-about-readmes.md`: Source: "About the repository README
  file", GitHub Docs, https://github.com/github/docs (content license: CC BY 4.0, see
  https://github.com/github/docs/blob/main/LICENSE). Copyright GitHub, Inc.

## Public domain and open data

- `text-*-gutenberg-*.txt`: Project Gutenberg texts, public domain in the USA. The Project
  Gutenberg License text is inside each file. `text-cp1252-derived-gutenberg-alice.txt` is ebook
  #11 transcoded to Windows-1252.
- `text-shiftjis-aozora-botchan.txt`: Aozora Bunko, public domain.
- `csv-datahub-co2-fossil-global.csv`: ODC Public Domain Dedication and License 1.0 (PDDL),
  https://opendatacommons.org/licenses/pddl/1-0/ . No notice is required.
- `csv-pandas-fdic-banklist.csv`, `csv-broken-derived-banklist.csv`: the data is from the US
  FDIC (public domain); the file comes from the pandas test data (BSD-3-Clause, text below).

## License texts of software sources

### pandas (BSD-3-Clause): `csv-pandas-fdic-banklist.csv`, `csv-broken-derived-banklist.csv`

Source of the text: https://raw.githubusercontent.com/pandas-dev/pandas/main/LICENSE

    BSD 3-Clause License

    Copyright (c) 2008-2011, AQR Capital Management, LLC, Lambda Foundry, Inc. and PyData Development Team
    All rights reserved.

    Copyright (c) 2011-2026, Open source contributors.

    Redistribution and use in source and binary forms, with or without
    modification, are permitted provided that the following conditions are met:

    * Redistributions of source code must retain the above copyright notice, this
      list of conditions and the following disclaimer.

    * Redistributions in binary form must reproduce the above copyright notice,
      this list of conditions and the following disclaimer in the documentation
      and/or other materials provided with the distribution.

    * Neither the name of the copyright holder nor the names of its
      contributors may be used to endorse or promote products derived from
      this software without specific prior written permission.

    THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
    AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
    IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
    DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
    FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
    DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
    SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
    CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
    OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
    OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

### Foam (MIT): `md-wikilinks-foam-docs.md`

Source of the text: https://raw.githubusercontent.com/foambubble/foam/main/LICENSE

    The MIT Licence (MIT)

    Copyright 2020 - present Jani Eväkallio <jani.evakallio@gmail.com>

    Permission is hereby granted, free of charge, to any person obtaining a copy of
    this software and associated documentation files (the "Software"), to deal in
    the Software without restriction, including without limitation the rights to
    use, copy, modify, merge, publish, distribute, sublicence, and/or sell copies of
    the Software, and to permit persons to whom the Software is furnished to do so,
    subject to the following conditions:

    The above copyright notice and this permission notice shall be included in all
    copies or substantial portions of the Software.

    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
    IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
    FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
    COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
    IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
    CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

    Where noted, some code uses the following license:

    MIT License

    Copyright (c) 2015 - present Microsoft Corporation

    Permission is hereby granted, free of charge, to any person obtaining a copy
    of this software and associated documentation files (the "Software"), to deal
    in the Software without restriction, including without limitation the rights
    to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
    copies of the Software, and to permit persons to whom the Software is
    furnished to do so, subject to the following conditions:

### Rust blog (MIT OR Apache-2.0): `md-toml-frontmatter-rust-blog-traits.md`

The file is used under the MIT license. Source of the text:
https://raw.githubusercontent.com/rust-lang/blog.rust-lang.org/main/LICENSE-MIT . The upstream
file has no copyright line; the blog's authors are named in the post (Aaron Turon).

    Permission is hereby granted, free of charge, to any
    person obtaining a copy of this software and associated
    documentation files (the "Software"), to deal in the
    Software without restriction, including without
    limitation the rights to use, copy, modify, merge,
    publish, distribute, sublicense, and/or sell copies of
    the Software, and to permit persons to whom the Software
    is furnished to do so, subject to the following
    conditions:

    The above copyright notice and this permission notice
    shall be included in all copies or substantial portions
    of the Software.

    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
    ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
    TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
    PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
    SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
    CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
    OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
    IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
    DEALINGS IN THE SOFTWARE.

### csv-spectrum (BSD-2-Clause): `csv-spectrum-*.csv`

Upstream (https://github.com/max-mapper/csv-spectrum) has no LICENSE file. Its `package.json`
(https://raw.githubusercontent.com/max-mapper/csv-spectrum/master/package.json) declares
`"license": "BSD-2-Clause"` and `"author": "max ogden"`. The standard BSD-2-Clause text with
that author as holder:

    BSD 2-Clause License

    Copyright (c) max ogden

    Redistribution and use in source and binary forms, with or without
    modification, are permitted provided that the following conditions are met:

    1. Redistributions of source code must retain the above copyright notice, this
       list of conditions and the following disclaimer.

    2. Redistributions in binary form must reproduce the above copyright notice,
       this list of conditions and the following disclaimer in the documentation
       and/or other materials provided with the distribution.

    THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
    AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
    IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
    DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
    FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
    DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
    SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
    CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
    OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
    OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

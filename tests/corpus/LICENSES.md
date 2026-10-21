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

## Spreadsheet corpus (`office/`)

The spreadsheet files were downloaded on 2026-10-20 from the raw form of the pinned commit URLs in `MANIFEST.md` and left byte for byte unchanged. Each SHA-256 was checked against the first 16 hex digits recorded in the spike manifest (`docs/spike-2026-10-19/MANIFEST-office.md`). License texts were fetched from the same pinned commits, except where noted.

### Apache-2.0 sources

- Apache POI test data (Apache-2.0): `office/Formatting.xlsx`, `55906-MultiSheetRefs.xlsx`, `MatrixFormulaEvalTestData.xlsx`, `simple-table-named-range.xlsx`, `testNames.xlsm`, `DataValidations-49244.xlsx`, `SimpleWithComments.xlsx`, `GroupTest.xlsx`, `WithConditionalFormatting.xlsx`, `WithChart.xlsx`. Source: https://github.com/apache/poi/tree/04804ac97487ceded07a8231bc106a258109cbeb/test-data/spreadsheet . Copyright The Apache Software Foundation. NOTICE: https://github.com/apache/poi/blob/04804ac97487ceded07a8231bc106a258109cbeb/legal/NOTICE . The license text is the Apache License 2.0 below.

- Apache Tika test documents (Apache-2.0): `office/testEXCEL_custom_props.xlsx`, `testEXCEL_various.xlsb`. Source: https://github.com/apache/tika/tree/043762c6327ff36b3081ce297688e93915c85996 . Copyright The Apache Software Foundation. NOTICE: https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/NOTICE.txt . The license text is the Apache License 2.0 below.

- Mustang project test data (Apache-2.0): `pdf/mustang-gnuaccounting-invoice-508.pdf` (renamed from `MustangGnuaccountingBeispielRE-20201121_508blanko.pdf`, bytes unchanged). Source: https://github.com/ZUGFeRD/mustangproject/tree/c83a73af4e57b45a8bb396ce97003f0fca05f5a9 . Copyright 2014 Jochen Staerk. NOTICE: https://github.com/ZUGFeRD/mustangproject/blob/c83a73af4e57b45a8bb396ce97003f0fca05f5a9/NOTICE . The license text is the Apache License 2.0 below.

NOTICE of Apache POI (section 4(d) of the license), source: https://raw.githubusercontent.com/apache/poi/04804ac97487ceded07a8231bc106a258109cbeb/legal/NOTICE

    Apache POI
    Copyright 2003-2026 The Apache Software Foundation

    This product includes software developed at
    The Apache Software Foundation (https://www.apache.org/).

    This product contains parts that were originally based on software from BEA.
    Copyright (c) 2000-2003, BEA Systems, <http://www.bea.com/> (dead link),
    which was acquired by Oracle Corporation in 2008.
    <http://www.oracle.com/us/corporate/Acquisitions/bea/index.html>
    <https://en.wikipedia.org/wiki/BEA_Systems>
    Note: The ASF Secretary has on hand a Software Grant Agreement (SGA) from
    BEA Systems, Inc. dated 9 Sep 2003 for XMLBeans signed by their EVP/CFO.

    This product contains W3C XML Schema documents. Copyright 2001-2003 (c)
    World Wide Web Consortium (Massachusetts Institute of Technology, European
    Research Consortium for Informatics and Mathematics, Keio University)

    This product contains the chunks_parse_cmds.tbl file from the vsdump program.
    Copyright (C) 2006-2007 Valek Filippov (frob@df.ru)

    This product contains parts of the eID Applet project
    <http://eid-applet.googlecode.com> and <https://github.com/e-Contract/eid-applet>.
    Copyright (c) 2009-2018
    FedICT (federal ICT department of Belgium), e-Contract.be BVBA (https://www.e-contract.be),
    Bart Hanssens from FedICT

    ExceptionUtils is derived from `scala.util.control.NonFatal` in scala-library
    which was released under the Apache 2.0 license.

    Copyright (c) 2002-2023 EPFL
    Copyright (c) 2011-2023 Lightbend, Inc.

    Scala includes software developed at
    LAMP/EPFL (https://lamp.epfl.ch/) and
    Lightbend, Inc. (https://www.lightbend.com/).

NOTICE of Apache Tika, source: https://raw.githubusercontent.com/apache/tika/043762c6327ff36b3081ce297688e93915c85996/NOTICE.txt

    Apache Tika
    Copyright 2007-2026 The Apache Software Foundation

    This product includes software developed at
    The Apache Software Foundation (http://www.apache.org/).

    Copyright 1993-2010 University Corporation for Atmospheric Research/Unidata
    This software contains code derived from UCAR/Unidata's NetCDF library.

    Tika-parsers component uses CDDL/LGPL dual-licensed dependency: jhighlight (https://github.com/codelibs/jhighlight)

    IPTC Photo Metadata descriptions Copyright 2010 International Press Telecommunications Council.

    Tika-mimetypes.xml includes mimetype definitions that were adapted from the PRONOM Technical Registry
    by The National Archives (http://www.nationalarchives.gov.uk/PRONOM/Default.aspx). PRONOM is published
    under the Open Government License 3.0 (http://www.nationalarchives.gov.uk/doc/open-government-licence/version/3/)

Apache License 2.0, source of the text: https://www.apache.org/licenses/LICENSE-2.0.txt

                                     Apache License
                               Version 2.0, January 2004
                            http://www.apache.org/licenses/

       TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

       1. Definitions.

          "License" shall mean the terms and conditions for use, reproduction,
          and distribution as defined by Sections 1 through 9 of this document.

          "Licensor" shall mean the copyright owner or entity authorized by
          the copyright owner that is granting the License.

          "Legal Entity" shall mean the union of the acting entity and all
          other entities that control, are controlled by, or are under common
          control with that entity. For the purposes of this definition,
          "control" means (i) the power, direct or indirect, to cause the
          direction or management of such entity, whether by contract or
          otherwise, or (ii) ownership of fifty percent (50%) or more of the
          outstanding shares, or (iii) beneficial ownership of such entity.

          "You" (or "Your") shall mean an individual or Legal Entity
          exercising permissions granted by this License.

          "Source" form shall mean the preferred form for making modifications,
          including but not limited to software source code, documentation
          source, and configuration files.

          "Object" form shall mean any form resulting from mechanical
          transformation or translation of a Source form, including but
          not limited to compiled object code, generated documentation,
          and conversions to other media types.

          "Work" shall mean the work of authorship, whether in Source or
          Object form, made available under the License, as indicated by a
          copyright notice that is included in or attached to the work
          (an example is provided in the Appendix below).

          "Derivative Works" shall mean any work, whether in Source or Object
          form, that is based on (or derived from) the Work and for which the
          editorial revisions, annotations, elaborations, or other modifications
          represent, as a whole, an original work of authorship. For the purposes
          of this License, Derivative Works shall not include works that remain
          separable from, or merely link (or bind by name) to the interfaces of,
          the Work and Derivative Works thereof.

          "Contribution" shall mean any work of authorship, including
          the original version of the Work and any modifications or additions
          to that Work or Derivative Works thereof, that is intentionally
          submitted to Licensor for inclusion in the Work by the copyright owner
          or by an individual or Legal Entity authorized to submit on behalf of
          the copyright owner. For the purposes of this definition, "submitted"
          means any form of electronic, verbal, or written communication sent
          to the Licensor or its representatives, including but not limited to
          communication on electronic mailing lists, source code control systems,
          and issue tracking systems that are managed by, or on behalf of, the
          Licensor for the purpose of discussing and improving the Work, but
          excluding communication that is conspicuously marked or otherwise
          designated in writing by the copyright owner as "Not a Contribution."

          "Contributor" shall mean Licensor and any individual or Legal Entity
          on behalf of whom a Contribution has been received by Licensor and
          subsequently incorporated within the Work.

       2. Grant of Copyright License. Subject to the terms and conditions of
          this License, each Contributor hereby grants to You a perpetual,
          worldwide, non-exclusive, no-charge, royalty-free, irrevocable
          copyright license to reproduce, prepare Derivative Works of,
          publicly display, publicly perform, sublicense, and distribute the
          Work and such Derivative Works in Source or Object form.

       3. Grant of Patent License. Subject to the terms and conditions of
          this License, each Contributor hereby grants to You a perpetual,
          worldwide, non-exclusive, no-charge, royalty-free, irrevocable
          (except as stated in this section) patent license to make, have made,
          use, offer to sell, sell, import, and otherwise transfer the Work,
          where such license applies only to those patent claims licensable
          by such Contributor that are necessarily infringed by their
          Contribution(s) alone or by combination of their Contribution(s)
          with the Work to which such Contribution(s) was submitted. If You
          institute patent litigation against any entity (including a
          cross-claim or counterclaim in a lawsuit) alleging that the Work
          or a Contribution incorporated within the Work constitutes direct
          or contributory patent infringement, then any patent licenses
          granted to You under this License for that Work shall terminate
          as of the date such litigation is filed.

       4. Redistribution. You may reproduce and distribute copies of the
          Work or Derivative Works thereof in any medium, with or without
          modifications, and in Source or Object form, provided that You
          meet the following conditions:

          (a) You must give any other recipients of the Work or
              Derivative Works a copy of this License; and

          (b) You must cause any modified files to carry prominent notices
              stating that You changed the files; and

          (c) You must retain, in the Source form of any Derivative Works
              that You distribute, all copyright, patent, trademark, and
              attribution notices from the Source form of the Work,
              excluding those notices that do not pertain to any part of
              the Derivative Works; and

          (d) If the Work includes a "NOTICE" text file as part of its
              distribution, then any Derivative Works that You distribute must
              include a readable copy of the attribution notices contained
              within such NOTICE file, excluding those notices that do not
              pertain to any part of the Derivative Works, in at least one
              of the following places: within a NOTICE text file distributed
              as part of the Derivative Works; within the Source form or
              documentation, if provided along with the Derivative Works; or,
              within a display generated by the Derivative Works, if and
              wherever such third-party notices normally appear. The contents
              of the NOTICE file are for informational purposes only and
              do not modify the License. You may add Your own attribution
              notices within Derivative Works that You distribute, alongside
              or as an addendum to the NOTICE text from the Work, provided
              that such additional attribution notices cannot be construed
              as modifying the License.

          You may add Your own copyright statement to Your modifications and
          may provide additional or different license terms and conditions
          for use, reproduction, or distribution of Your modifications, or
          for any such Derivative Works as a whole, provided Your use,
          reproduction, and distribution of the Work otherwise complies with
          the conditions stated in this License.

       5. Submission of Contributions. Unless You explicitly state otherwise,
          any Contribution intentionally submitted for inclusion in the Work
          by You to the Licensor shall be under the terms and conditions of
          this License, without any additional terms or conditions.
          Notwithstanding the above, nothing herein shall supersede or modify
          the terms of any separate license agreement you may have executed
          with Licensor regarding such Contributions.

       6. Trademarks. This License does not grant permission to use the trade
          names, trademarks, service marks, or product names of the Licensor,
          except as required for reasonable and customary use in describing the
          origin of the Work and reproducing the content of the NOTICE file.

       7. Disclaimer of Warranty. Unless required by applicable law or
          agreed to in writing, Licensor provides the Work (and each
          Contributor provides its Contributions) on an "AS IS" BASIS,
          WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
          implied, including, without limitation, any warranties or conditions
          of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
          PARTICULAR PURPOSE. You are solely responsible for determining the
          appropriateness of using or redistributing the Work and assume any
          risks associated with Your exercise of permissions under this License.

       8. Limitation of Liability. In no event and under no legal theory,
          whether in tort (including negligence), contract, or otherwise,
          unless required by applicable law (such as deliberate and grossly
          negligent acts) or agreed to in writing, shall any Contributor be
          liable to You for damages, including any direct, indirect, special,
          incidental, or consequential damages of any character arising as a
          result of this License or out of the use or inability to use the
          Work (including but not limited to damages for loss of goodwill,
          work stoppage, computer failure or malfunction, or any and all
          other commercial damages or losses), even if such Contributor
          has been advised of the possibility of such damages.

       9. Accepting Warranty or Additional Liability. While redistributing
          the Work or Derivative Works thereof, You may choose to offer,
          and charge a fee for, acceptance of support, warranty, indemnity,
          or other liability obligations and/or rights consistent with this
          License. However, in accepting such obligations, You may act only
          on Your own behalf and on Your sole responsibility, not on behalf
          of any other Contributor, and only if You agree to indemnify,
          defend, and hold each Contributor harmless for any liability
          incurred by, or claims asserted against, such Contributor by reason
          of your accepting any such warranty or additional liability.

       END OF TERMS AND CONDITIONS

       APPENDIX: How to apply the Apache License to your work.

          To apply the Apache License to your work, attach the following
          boilerplate notice, with the fields enclosed by brackets "[]"
          replaced with your own identifying information. (Don't include
          the brackets!)  The text should be enclosed in the appropriate
          comment syntax for the file format. We also recommend that a
          file or class name and description of purpose be included on the
          same "printed page" as the copyright notice for easier
          identification within third-party archives.

       Copyright [yyyy] [name of copyright owner]

       Licensed under the Apache License, Version 2.0 (the "License");
       you may not use this file except in compliance with the License.
       You may obtain a copy of the License at

           http://www.apache.org/licenses/LICENSE-2.0

       Unless required by applicable law or agreed to in writing, software
       distributed under the License is distributed on an "AS IS" BASIS,
       WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
       See the License for the specific language governing permissions and
       limitations under the License.


### LibreOffice core (MPL-2.0): `office/threadedComment.xlsx`, `hidden_sheets.xlsx`, `external-refs.xlsx`, `formula-across-sheets.ods`, `one-variable.xlsx`

Source: https://github.com/LibreOffice/core/tree/79be16a7156fadfad7d4a0be0dbbb38f761f242e/sc/qa/unit/data . Source of the text: https://raw.githubusercontent.com/LibreOffice/core/79be16a7156fadfad7d4a0be0dbbb38f761f242e/COPYING.MPL . The files are unchanged, so the MPL notice applies to them as Source Code Form.

    Mozilla Public License Version 2.0
    ==================================

    1. Definitions
    --------------

    1.1. "Contributor"
        means each individual or legal entity that creates, contributes to
        the creation of, or owns Covered Software.

    1.2. "Contributor Version"
        means the combination of the Contributions of others (if any) used
        by a Contributor and that particular Contributor's Contribution.

    1.3. "Contribution"
        means Covered Software of a particular Contributor.

    1.4. "Covered Software"
        means Source Code Form to which the initial Contributor has attached
        the notice in Exhibit A, the Executable Form of such Source Code
        Form, and Modifications of such Source Code Form, in each case
        including portions thereof.

    1.5. "Incompatible With Secondary Licenses"
        means

        (a) that the initial Contributor has attached the notice described
            in Exhibit B to the Covered Software; or

        (b) that the Covered Software was made available under the terms of
            version 1.1 or earlier of the License, but not also under the
            terms of a Secondary License.

    1.6. "Executable Form"
        means any form of the work other than Source Code Form.

    1.7. "Larger Work"
        means a work that combines Covered Software with other material, in
        a separate file or files, that is not Covered Software.

    1.8. "License"
        means this document.

    1.9. "Licensable"
        means having the right to grant, to the maximum extent possible,
        whether at the time of the initial grant or subsequently, any and
        all of the rights conveyed by this License.

    1.10. "Modifications"
        means any of the following:

        (a) any file in Source Code Form that results from an addition to,
            deletion from, or modification of the contents of Covered
            Software; or

        (b) any new file in Source Code Form that contains any Covered
            Software.

    1.11. "Patent Claims" of a Contributor
        means any patent claim(s), including without limitation, method,
        process, and apparatus claims, in any patent Licensable by such
        Contributor that would be infringed, but for the grant of the
        License, by the making, using, selling, offering for sale, having
        made, import, or transfer of either its Contributions or its
        Contributor Version.

    1.12. "Secondary License"
        means either the GNU General Public License, Version 2.0, the GNU
        Lesser General Public License, Version 2.1, the GNU Affero General
        Public License, Version 3.0, or any later versions of those
        licenses.

    1.13. "Source Code Form"
        means the form of the work preferred for making modifications.

    1.14. "You" (or "Your")
        means an individual or a legal entity exercising rights under this
        License. For legal entities, "You" includes any entity that
        controls, is controlled by, or is under common control with You. For
        purposes of this definition, "control" means (a) the power, direct
        or indirect, to cause the direction or management of such entity,
        whether by contract or otherwise, or (b) ownership of more than
        fifty percent (50%) of the outstanding shares or beneficial
        ownership of such entity.

    2. License Grants and Conditions
    --------------------------------

    2.1. Grants

    Each Contributor hereby grants You a world-wide, royalty-free,
    non-exclusive license:

    (a) under intellectual property rights (other than patent or trademark)
        Licensable by such Contributor to use, reproduce, make available,
        modify, display, perform, distribute, and otherwise exploit its
        Contributions, either on an unmodified basis, with Modifications, or
        as part of a Larger Work; and

    (b) under Patent Claims of such Contributor to make, use, sell, offer
        for sale, have made, import, and otherwise transfer either its
        Contributions or its Contributor Version.

    2.2. Effective Date

    The licenses granted in Section 2.1 with respect to any Contribution
    become effective for each Contribution on the date the Contributor first
    distributes such Contribution.

    2.3. Limitations on Grant Scope

    The licenses granted in this Section 2 are the only rights granted under
    this License. No additional rights or licenses will be implied from the
    distribution or licensing of Covered Software under this License.
    Notwithstanding Section 2.1(b) above, no patent license is granted by a
    Contributor:

    (a) for any code that a Contributor has removed from Covered Software;
        or

    (b) for infringements caused by: (i) Your and any other third party's
        modifications of Covered Software, or (ii) the combination of its
        Contributions with other software (except as part of its Contributor
        Version); or

    (c) under Patent Claims infringed by Covered Software in the absence of
        its Contributions.

    This License does not grant any rights in the trademarks, service marks,
    or logos of any Contributor (except as may be necessary to comply with
    the notice requirements in Section 3.4).

    2.4. Subsequent Licenses

    No Contributor makes additional grants as a result of Your choice to
    distribute the Covered Software under a subsequent version of this
    License (see Section 10.2) or under the terms of a Secondary License (if
    permitted under the terms of Section 3.3).

    2.5. Representation

    Each Contributor represents that the Contributor believes its
    Contributions are its original creation(s) or it has sufficient rights
    to grant the rights to its Contributions conveyed by this License.

    2.6. Fair Use

    This License is not intended to limit any rights You have under
    applicable copyright doctrines of fair use, fair dealing, or other
    equivalents.

    2.7. Conditions

    Sections 3.1, 3.2, 3.3, and 3.4 are conditions of the licenses granted
    in Section 2.1.

    3. Responsibilities
    -------------------

    3.1. Distribution of Source Form

    All distribution of Covered Software in Source Code Form, including any
    Modifications that You create or to which You contribute, must be under
    the terms of this License. You must inform recipients that the Source
    Code Form of the Covered Software is governed by the terms of this
    License, and how they can obtain a copy of this License. You may not
    attempt to alter or restrict the recipients' rights in the Source Code
    Form.

    3.2. Distribution of Executable Form

    If You distribute Covered Software in Executable Form then:

    (a) such Covered Software must also be made available in Source Code
        Form, as described in Section 3.1, and You must inform recipients of
        the Executable Form how they can obtain a copy of such Source Code
        Form by reasonable means in a timely manner, at a charge no more
        than the cost of distribution to the recipient; and

    (b) You may distribute such Executable Form under the terms of this
        License, or sublicense it under different terms, provided that the
        license for the Executable Form does not attempt to limit or alter
        the recipients' rights in the Source Code Form under this License.

    3.3. Distribution of a Larger Work

    You may create and distribute a Larger Work under terms of Your choice,
    provided that You also comply with the requirements of this License for
    the Covered Software. If the Larger Work is a combination of Covered
    Software with a work governed by one or more Secondary Licenses, and the
    Covered Software is not Incompatible With Secondary Licenses, this
    License permits You to additionally distribute such Covered Software
    under the terms of such Secondary License(s), so that the recipient of
    the Larger Work may, at their option, further distribute the Covered
    Software under the terms of either this License or such Secondary
    License(s).

    3.4. Notices

    You may not remove or alter the substance of any license notices
    (including copyright notices, patent notices, disclaimers of warranty,
    or limitations of liability) contained within the Source Code Form of
    the Covered Software, except that You may alter any license notices to
    the extent required to remedy known factual inaccuracies.

    3.5. Application of Additional Terms

    You may choose to offer, and to charge a fee for, warranty, support,
    indemnity or liability obligations to one or more recipients of Covered
    Software. However, You may do so only on Your own behalf, and not on
    behalf of any Contributor. You must make it absolutely clear that any
    such warranty, support, indemnity, or liability obligation is offered by
    You alone, and You hereby agree to indemnify every Contributor for any
    liability incurred by such Contributor as a result of warranty, support,
    indemnity or liability terms You offer. You may include additional
    disclaimers of warranty and limitations of liability specific to any
    jurisdiction.

    4. Inability to Comply Due to Statute or Regulation
    ---------------------------------------------------

    If it is impossible for You to comply with any of the terms of this
    License with respect to some or all of the Covered Software due to
    statute, judicial order, or regulation then You must: (a) comply with
    the terms of this License to the maximum extent possible; and (b)
    describe the limitations and the code they affect. Such description must
    be placed in a text file included with all distributions of the Covered
    Software under this License. Except to the extent prohibited by statute
    or regulation, such description must be sufficiently detailed for a
    recipient of ordinary skill to be able to understand it.

    5. Termination
    --------------

    5.1. The rights granted under this License will terminate automatically
    if You fail to comply with any of its terms. However, if You become
    compliant, then the rights granted under this License from a particular
    Contributor are reinstated (a) provisionally, unless and until such
    Contributor explicitly and finally terminates Your grants, and (b) on an
    ongoing basis, if such Contributor fails to notify You of the
    non-compliance by some reasonable means prior to 60 days after You have
    come back into compliance. Moreover, Your grants from a particular
    Contributor are reinstated on an ongoing basis if such Contributor
    notifies You of the non-compliance by some reasonable means, this is the
    first time You have received notice of non-compliance with this License
    from such Contributor, and You become compliant prior to 30 days after
    Your receipt of the notice.

    5.2. If You initiate litigation against any entity by asserting a patent
    infringement claim (excluding declaratory judgment actions,
    counter-claims, and cross-claims) alleging that a Contributor Version
    directly or indirectly infringes any patent, then the rights granted to
    You by any and all Contributors for the Covered Software under Section
    2.1 of this License shall terminate.

    5.3. In the event of termination under Sections 5.1 or 5.2 above, all
    end user license agreements (excluding distributors and resellers) which
    have been validly granted by You or Your distributors under this License
    prior to termination shall survive termination.

    ************************************************************************
    *                                                                      *
    *  6. Disclaimer of Warranty                                           *
    *  -------------------------                                           *
    *                                                                      *
    *  Covered Software is provided under this License on an "as is"       *
    *  basis, without warranty of any kind, either expressed, implied, or  *
    *  statutory, including, without limitation, warranties that the       *
    *  Covered Software is free of defects, merchantable, fit for a        *
    *  particular purpose or non-infringing. The entire risk as to the     *
    *  quality and performance of the Covered Software is with You.        *
    *  Should any Covered Software prove defective in any respect, You     *
    *  (not any Contributor) assume the cost of any necessary servicing,   *
    *  repair, or correction. This disclaimer of warranty constitutes an   *
    *  essential part of this License. No use of any Covered Software is   *
    *  authorized under this License except under this disclaimer.         *
    *                                                                      *
    ************************************************************************

    ************************************************************************
    *                                                                      *
    *  7. Limitation of Liability                                          *
    *  --------------------------                                          *
    *                                                                      *
    *  Under no circumstances and under no legal theory, whether tort      *
    *  (including negligence), contract, or otherwise, shall any           *
    *  Contributor, or anyone who distributes Covered Software as          *
    *  permitted above, be liable to You for any direct, indirect,         *
    *  special, incidental, or consequential damages of any character      *
    *  including, without limitation, damages for lost profits, loss of    *
    *  goodwill, work stoppage, computer failure or malfunction, or any    *
    *  and all other commercial damages or losses, even if such party      *
    *  shall have been informed of the possibility of such damages. This   *
    *  limitation of liability shall not apply to liability for death or   *
    *  personal injury resulting from such party's negligence to the       *
    *  extent applicable law prohibits such limitation. Some               *
    *  jurisdictions do not allow the exclusion or limitation of           *
    *  incidental or consequential damages, so this exclusion and          *
    *  limitation may not apply to You.                                    *
    *                                                                      *
    ************************************************************************

    8. Litigation
    -------------

    Any litigation relating to this License may be brought only in the
    courts of a jurisdiction where the defendant maintains its principal
    place of business and such litigation shall be governed by laws of that
    jurisdiction, without reference to its conflict-of-law provisions.
    Nothing in this Section shall prevent a party's ability to bring
    cross-claims or counter-claims.

    9. Miscellaneous
    ----------------

    This License represents the complete agreement concerning the subject
    matter hereof. If any provision of this License is held to be
    unenforceable, such provision shall be reformed only to the extent
    necessary to make it enforceable. Any law or regulation which provides
    that the language of a contract shall be construed against the drafter
    shall not be used to construe this License against a Contributor.

    10. Versions of the License
    ---------------------------

    10.1. New Versions

    Mozilla Foundation is the license steward. Except as provided in Section
    10.3, no one other than the license steward has the right to modify or
    publish new versions of this License. Each version will be given a
    distinguishing version number.

    10.2. Effect of New Versions

    You may distribute the Covered Software under the terms of the version
    of the License under which You originally received the Covered Software,
    or under the terms of any subsequent version published by the license
    steward.

    10.3. Modified Versions

    If you create software not governed by this License, and you want to
    create a new license for such software, you may create and use a
    modified version of this License if you rename the license and remove
    any references to the name of the license steward (except to note that
    such modified license differs from this License).

    10.4. Distributing Source Code Form that is Incompatible With Secondary
    Licenses

    If You choose to distribute Source Code Form that is Incompatible With
    Secondary Licenses under the terms of this version of the License, the
    notice described in Exhibit B of this License must be attached.

    Exhibit A - Source Code Form License Notice
    -------------------------------------------

      This Source Code Form is subject to the terms of the Mozilla Public
      License, v. 2.0. If a copy of the MPL was not distributed with this
      file, You can obtain one at http://mozilla.org/MPL/2.0/.

    If it is not possible or desirable to put the notice in a particular
    file, then You may include the notice in a location (such as a LICENSE
    file in a relevant directory) where a recipient would be likely to look
    for such a notice.

    You may add additional accurate notices of copyright ownership.

    Exhibit B - "Incompatible With Secondary Licenses" Notice
    ---------------------------------------------------------

      This Source Code Form is "Incompatible With Secondary Licenses", as
      defined by the Mozilla Public License, v. 2.0.


### calamine (MIT): `office/merged_range.xlsx`, `table-multiple.xlsx`, `pivots.xlsx`, `any_sheets.ods`, `with-annotation.ods`, `malformed/issue_666_panic.xlsb`, `malformed/issue_594_dos.ods`

Source of the text: https://raw.githubusercontent.com/tafia/calamine/f2c5948242fdfe390e292313a1f87f9db19b9633/LICENSE-MIT.md

    The MIT License (MIT)

    Copyright (c) 2016 Johann Tuffe

    Permission is hereby granted, free of charge, to any person obtaining a copy
    of this software and associated documentation files (the "Software"), to deal
    in the Software without restriction, including without limitation the rights
    to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
    copies of the Software, and to permit persons to whom the Software is
    furnished to do so, subject to the following conditions:


    The above copyright notice and this permission notice shall be included in
    all copies or substantial portions of the Software.


    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
    IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
    FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.  IN NO EVENT SHALL THE
    AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
    LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
    OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
    THE SOFTWARE.


### IronCalc (MIT OR Apache-2.0, used under MIT): `office/CONVERT.xlsx`, `XLOOKUP.xlsx`, `LET.xlsx`, `LAMBDA.xlsx`, `DynamicArrays.xlsx`

Source of the text: https://raw.githubusercontent.com/ironcalc/IronCalc/e94e6f1aed1e9a4709f038717ca73d9d82e63ab5/LICENSE-MIT

    MIT License

    Copyright (c) 2023 EqualTo GmbH, 2023 Nicolás Hatcher

    Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

    The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.


### unifloc_vba (MIT): `office/ex051.MF_pipeline.xlsx`, `ex054.MF_friction_factor.xlsm`, `ex012.PVT_z_factor.xlsm`

Source of the text: https://raw.githubusercontent.com/unifloc/unifloc_vba/b19a2b0715d718ecbbd51a0c84ccda611214dfc6/LICENSE

    MIT License

    Copyright (c) 2017 Rinat Khabibullin

    Permission is hereby granted, free of charge, to any person obtaining a copy
    of this software and associated documentation files (the "Software"), to deal
    in the Software without restriction, including without limitation the rights
    to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
    copies of the Software, and to permit persons to whom the Software is
    furnished to do so, subject to the following conditions:

    The above copyright notice and this permission notice shall be included in all
    copies or substantial portions of the Software.

    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
    IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
    FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
    AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
    LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
    OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
    SOFTWARE.


### underground-tunnel-structural-analysis (MIT): `office/tunnel_design_calculations.xlsx`

Source of the text: https://raw.githubusercontent.com/alokpalai/underground-tunnel-structural-analysis/5063b5488ea2b983daeb7b4200d2fbdab0dcac6b/LICENSE

    MIT License

    Copyright (c) 2026 Alok Pal

    Permission is hereby granted, free of charge, to any person obtaining a copy
    of this software and associated documentation files (the "Software"), to deal
    in the Software without restriction, including without limitation the rights
    to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
    copies of the Software, and to permit persons to whom the Software is
    furnished to do so, subject to the following conditions:

    The above copyright notice and this permission notice shall be included in all
    copies or substantial portions of the Software.

    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
    IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
    FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
    AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
    LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
    OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
    SOFTWARE.

## PDF corpus (`pdf/`)

The PDF files were downloaded on 2026-10-19 (and checked again on 2026-10-20; the two pdfplumber files on 2026-10-21) from the raw form of the pinned commit URLs in `MANIFEST.md` and left byte for byte unchanged. Each SHA-256 was checked against `docs/spike-2026-10-19/MANIFEST-pdf.md`. The two large files that are only in `remote.tsv` are not in the repo.

### Attribution and share-alike license (CC BY-SA 4.0)

License text: https://creativecommons.org/licenses/by-sa/4.0/legalcode . Source: the py-pdf sample-files repository, https://github.com/py-pdf/sample-files/tree/02b8203bf416db3dbdb03438bc8278dfcb0555f3 . The README of the repository says: "Every PDF in this repository is under the Creative Commons Attribution-ShareAlike License (CC-BY-SA-4.0)." The files are not changed. Authors are the contributors of the repository, named in its history and in the document properties of each file. Files: `pdf/pypdf-annotated.pdf`, `pypdf-arabic-habibi.pdf`, `pypdf-attachment.pdf`, `pypdf-libreoffice-form.pdf`, `pypdf-multicolumn.pdf`, `pypdf-outline.pdf`, `pypdf-xmp.pdf`, `pypdf-ocg-layers.pdf`. The share-alike term applies to these files and to adaptations of them, not to Rezzer code that only reads them.

### Attribution license (CC BY 4.0)

License text: https://creativecommons.org/licenses/by/4.0/legalcode . `pdf/arxiv-2408.09869-docling-report.pdf` (remote only): "Docling Technical Report", arXiv:2408.09869v5, https://arxiv.org/abs/2408.09869 , by Christoph Auer and others (IBM Research), licensed CC BY 4.0 according to the arXiv record. The file is not changed.

### Public domain

- `pdf/irs-f1040-2025.pdf`: Form 1040 (2025), Internal Revenue Service, https://www.irs.gov/pub/irs-pdf/f1040.pdf . Work of the US government, public domain.
- `pdf/nist-sp800-53r5.pdf` (remote only): NIST Special Publication 800-53 Revision 5, https://nvlpubs.nist.gov/nistpubs/SpecialPublications/NIST.SP.800-53r5.pdf . Work of the US government (NIST), public domain.
- `pdf/pdfbox-sf1199a-direct-deposit.pdf`: Standard Form 1199A, US Department of the Treasury. Work of the US government, public domain. It was found in the Apache PDFBox test resources.
- `pdf/scan-truman-letter-nara.pdf`: letter from Harry S. Truman to Eleanor Roosevelt, National Archives and Records Administration via the Digital Public Library of America, public domain on Wikimedia Commons (page named in `MANIFEST.md`).
- `pdf/senate-expenditures.pdf`: report of the expenditures of a US Senate office, https://github.com/jsvine/pdfplumber/blob/4c64b92d5caccd71c645e98e0fabb0c4dba7ff45/tests/pdfs/senate-expenditures.pdf . Work of the US government, public domain. It was found in the pdfplumber test files; the MIT license of that repository does not cover them, so the file is used for its public domain status.
- `pdf/nics-background-checks-2015-11.pdf`: NICS firearm background checks, November 2015, Federal Bureau of Investigation, https://github.com/jsvine/pdfplumber/blob/4c64b92d5caccd71c645e98e0fabb0c4dba7ff45/tests/pdfs/nics-background-checks-2015-11.pdf . Work of the US government, public domain. It was found in the pdfplumber test files, as above.
- `pdf/pdfbox-poems-article-threads.pdf` has public domain poems (Goethe and others) as text. The file itself is an Apache PDFBox test resource (Apache-2.0).

### Apache-2.0 sources

The license text is the Apache License 2.0 above (section "Spreadsheet corpus").

- pdf.js test files (Apache-2.0): `pdf/pdfjs-*.pdf`. Source: https://github.com/mozilla/pdf.js/tree/33734f1fdf1b8987c656c0a5f67b4e2cc57e9d13/test/pdfs . Copyright Mozilla and individual contributors; the annotation files were made for the pdf.js test suite by Tim van der Meij. The repository has no NOTICE file at this commit (https://raw.githubusercontent.com/mozilla/pdf.js/33734f1fdf1b8987c656c0a5f67b4e2cc57e9d13/NOTICE returns "not found"), so there is no notice text to include. License: https://raw.githubusercontent.com/mozilla/pdf.js/33734f1fdf1b8987c656c0a5f67b4e2cc57e9d13/LICENSE .
- qpdf test files (Apache-2.0): `pdf/qpdf-*.pdf`. Source: https://github.com/qpdf/qpdf/tree/4eba95899886e851cc41d76886483b347612f2a8/qpdf/qtest/qpdf . Copyright Jay Berkenbilt and Manfred Holger. NOTICE: `NOTICE.md` of the repository, reproduced below.
- Apache PDFBox test resources (Apache-2.0): `pdf/pdfbox-*.pdf`. Source: https://github.com/apache/pdfbox/tree/ccb7498b942d99d7644f4c1d14906b646f814d84 . Copyright The Apache Software Foundation. NOTICE: `NOTICE.txt` of the repository, reproduced below.

NOTICE of qpdf (section 4(d) of the license), source: https://raw.githubusercontent.com/qpdf/qpdf/4eba95899886e851cc41d76886483b347612f2a8/NOTICE.md

    qpdf is copyright (c) 2005-2021 Jay Berkenbilt, 2022-2026 Jay Berkenbilt and Manfred Holger
    
    Licensed under the Apache License, Version 2.0 (the "License"); you may not use this file except in compliance with the License. You may obtain a copy of the License at
    
      http://www.apache.org/licenses/LICENSE-2.0
    
    Unless required by applicable law or agreed to in writing, software distributed under the License is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. See the License for the specific language governing permissions and limitations under the License.
    
    Versions of qpdf prior to version 7 were released under the terms of version 2.0 of the Artistic License. At your option, you may continue to consider qpdf to be licensed under those terms. Please see the manual for additional information.
    
    The qpdf distribution includes a copy of [qtest](https://qtest.sourceforge.io), which is released under the terms of the [version 2.0 of the Artistic license](https://opensource.org/licenses/Artistic-2.0), which can be found at https://opensource.org/licenses/Artistic-2.0.
    
    The Rijndael encryption implementation used as the basis for AES encryption and decryption support comes from Philip J. Erdelsky's public domain implementation.  The files `libqpdf/rijndael.cc` and `libqpdf/qpdf/rijndael.h` remain in the public domain.  They were obtained from
    * http://www.efgh.com/software/rijndael.htm
    * http://www.efgh.com/software/rijndael.txt
    
    The embedded sha2 code comes from sphlib 3.0
    * http://www.saphir2.com/sphlib/
    
    That code has the following license:
      ```
      Copyright (c) 2007-2011  Projet RNRT SAPHIR
    
      Permission is hereby granted, free of charge, to any person obtaining
      a copy of this software and associated documentation files (the
      "Software"), to deal in the Software without restriction, including
      without limitation the rights to use, copy, modify, merge, publish,
      distribute, sublicense, and/or sell copies of the Software, and to
      permit persons to whom the Software is furnished to do so, subject to
      the following conditions:
    
      The above copyright notice and this permission notice shall be included
      in all copies or substantial portions of the Software.
    
      THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
      EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
      MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
      IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
      CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
      TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
      SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
      ```

NOTICE of Apache PDFBox (section 4(d) of the license), source: https://raw.githubusercontent.com/apache/pdfbox/ccb7498b942d99d7644f4c1d14906b646f814d84/NOTICE.txt

    Apache PDFBox
    Copyright 2014 The Apache Software Foundation
    
    This product includes software developed at
    The Apache Software Foundation (http://www.apache.org/).
    
    Based on source code originally developed in the PDFBox and 
    FontBox projects.
    
    Copyright (c) 2002-2007, www.pdfbox.org
    
    Based on source code originally developed in the PaDaF project.
    Copyright (c) 2010 Atos Worldline SAS
    
    Includes the Adobe Glyph List
    Copyright 1997, 1998, 2002, 2007, 2010 Adobe Systems Incorporated.
    
    Includes the Zapf Dingbats Glyph List
    Copyright 2002, 2010 Adobe Systems Incorporated.
    
    Includes OSXAdapter
    Copyright (C) 2003-2007 Apple, Inc., All Rights Reserved

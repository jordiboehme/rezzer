# layoutregions

layoutregions is the region engine for pages that have text runs, rules, boxes and pictures.

A format reader hands over one page. The page lists its text runs, rules, boxes, pictures and
codes. It can also list regions that the file states itself. The crate gives back regions. Each
region has a kind, its bounds, the pieces it holds and the signals that decided its kind.

A piece belongs to one region only. When two finders want the same piece, one rule settles it.
For now the first finder wins. A later step adds a scored rule and records the loser as an
alternative.

Every limit has a default: regions per page, depth of the cut, alternatives per segment and
candidates per page. When a limit is hit, the crate says so in an issue and never drops
something without a word.

The crate has no dependencies. It does not know any file format.

License: MPL-2.0

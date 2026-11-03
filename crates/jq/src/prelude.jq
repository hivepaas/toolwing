# jq 1.8.2's own definitions (src/builtin.jq; jq is copyright (C) 2012
# Stephen Dolan, MIT license in LICENSE-jq), where jaq 3.1's differ or are
# missing (docs/design/jq.md). Loaded after jaq's, so these are the ones a
# program sees.

# jaq's number definitions, bound to a `nan` and `infinite` made without
# dividing by zero, which jq refuses (and so does the value type now).
def infinite: 1e1000 + 0;
def nan: infinite - infinite;
def isnan:      . < nan and nan < .;
def isinfinite: . == infinite or  . == -infinite;
def isfinite:   isnumber and (isinfinite | not);
def isnormal:   isnumber and ((. == 0 or isnan or isinfinite) | not);
def logb:
    if . == 0.0 then -infinite
  elif isinfinite then infinite
  elif isnan then .
  else ilogb | . + 0.0 end;
def significand:
    if isinfinite or isnan then .
  elif . == 0.0 then 0.0
  else scalbln(.; ilogb | -1 * .) end;
# Natives (natives.rs).
def @base64d: tostring | _toolwing_base64d;
def @urid: tostring | _toolwing_urid;
# Regex: jq's own definitions on `_match_impl`, Oniguruma (regex.rs).
def match(re; mode): _match_impl(re; mode; false)|.[];
def match($val): ($val|type) as $vt | if $vt == "string" then match($val; null)
   elif $vt == "array" and ($val | length) > 1 then match($val[0]; $val[1])
   elif $vt == "array" and ($val | length) > 0 then match($val[0]; null)
   else error( $vt + " not a string or array") end;
def test(re; mode): _match_impl(re; mode; true);
def test($val): ($val|type) as $vt | if $vt == "string" then test($val; null)
   elif $vt == "array" and ($val | length) > 1 then test($val[0]; $val[1])
   elif $vt == "array" and ($val | length) > 0 then test($val[0]; null)
   else error( $vt + " not a string or array") end;
def capture(re; mods): match(re; mods) | reduce ( .captures | .[] | select(.name != null) | { (.name) : .string } ) as $pair ({}; . + $pair);
def capture($val): ($val|type) as $vt | if $vt == "string" then capture($val; null)
   elif $vt == "array" and ($val | length) > 1 then capture($val[0]; $val[1])
   elif $vt == "array" and ($val | length) > 0 then capture($val[0]; null)
   else error( $vt + " not a string or array") end;
def scan($re; $flags):
  match($re; "g" + $flags)
    | if (.captures|length > 0)
      then [ .captures | .[] | .string ]
      else .string
      end;
def scan($re): scan($re; null);
def splits($re; $flags):
  .[foreach (match($re; $flags+"g"), null) as {$offset, $length}
      (null; {start: .next, end: $offset, next: ($offset+$length)})];
def splits($re): splits($re; null);
def split($re; $flags): [ splits($re; $flags) ];
def sub($re; s; $flags):
   . as $in
   | (reduce match($re; $flags) as $edit
        ({result: [], previous: 0};
            $in[ .previous: ($edit | .offset) ] as $gap
            | [reduce ( $edit | .captures | .[] | select(.name != null) | { (.name) : .string } ) as $pair
                 ({}; . + $pair) | s ] as $inserts
            | reduce range(0; $inserts|length) as $ix (.; .result[$ix] += $gap + $inserts[$ix])
            | .previous = ($edit | .offset + .length ) )
          | .result[] + $in[.previous:] )
      // $in;
def sub($re; s): sub($re; s; "");
def gsub($re; s; flags): sub($re; s; flags + "g");
def gsub($re; s): sub($re; s; "g");
def have_literal_numbers: true;
def have_decnum: false;
def _del1($p):
  if ($p | length) == 1 then del(.[$p[0]])
  elif . == null then .
  else .[$p[0]] |= _del1($p[1:]) end;
def delpaths($ps): reduce ($ps | unique | reverse | .[]) as $p (.; _del1($p));
def del(f): delpaths([path(f)]);
def from_entries: map({ (.key // .Key // .name // .Name):
  if has("value") then .value else .Value end }) | add // {};
def with_entries(f): to_entries | map(f) | from_entries;
def join($x): reduce .[] as $i (null;
            (if .==null then "" else .+$x end) +
            ($i | if type=="boolean" or type=="number" then tostring else .//"" end)
        ) // "";
def _flatten($x): reduce .[] as $i ([]; if $i | type == "array" and $x != 0 then . + ($i | _flatten($x-1)) else . + [$i] end);
def flatten($x): if $x < 0 then error("flatten depth must not be negative") else _flatten($x) end;
def flatten: _flatten(-1);
def trimstr($val): ltrimstr($val) | rtrimstr($val);
def limit($n; expr):
  if $n > 0 then label $out | foreach expr as $item ($n; . - 1; $item, if . <= 0 then break $out else empty end)
  elif $n == 0 then empty
  else error("limit doesn't support negative count") end;
def skip($n; expr):
  if $n > 0 then foreach expr as $item ($n; . - 1; if . < 0 then $item else empty end)
  elif $n == 0 then expr
  else error("skip doesn't support negative count") end;
def nth($n; g):
  if $n < 0 then error("nth doesn't support negative indices")
  else first(skip($n; g)) end;
def truncate_stream(stream):
  . as $n | null | stream | . as $input | if (.[0]|length) > $n then setpath([0];$input[0][$n:]) else empty end;
def fromstream(i): {x: null, e: false} as $init |
  foreach i as $i ($init
  ; if .e then $init else . end
  | if $i|length == 2
    then setpath(["e"]; $i[0]|length==0) | setpath(["x"]+$i[0]; $i[1])
    else setpath(["e"]; $i[0]|length==1) end
  ; if .e then .x else empty end);
def tostream:
  path(def r: (.[]?|r), .; r) as $p |
  getpath($p) |
  reduce path(.[]?) as $q ([$p, .]; [$p+$q]);
def pick(pathexps):
  . as $in
  | reduce path(pathexps) as $a (null;
      setpath($a; $in|getpath($a)) );
def INDEX(stream; idx_expr):
  reduce stream as $row ({}; .[$row|idx_expr|tostring] = $row);
def INDEX(idx_expr): INDEX(.[]; idx_expr);
def JOIN($idx; idx_expr):
  [.[] | [., $idx[idx_expr]]];
def JOIN($idx; stream; idx_expr):
  stream | [., $idx[idx_expr]];
def JOIN($idx; stream; idx_expr; join_expr):
  stream | [., $idx[idx_expr]] | join_expr;
def IN(s): any(s == .; .);
def IN(src; s): any(src == s; .);

# jq 1.8.2's definitions (src/builtin.jq; jq is copyright (C) 2012 Stephen
# Dolan, MIT license in LICENSE-jq) where jaq 3.1 differs or has none. A prototype for
# measuring (docs/design/jq.md); setpath, delpaths and del here stand in for
# the value type's own behaviour.
def have_literal_numbers: true;
def have_decnum: false;
def setpath($p; $v):
  if ($p | length) == 0 then $v
  else $p[0] as $k
  | (if . == null then (if ($k | type) == "number" then [] else {} end) else . end)
  | if type == "array" and ($k | type) == "number" and $k >= length
    then . + [range(length; $k + 1) | null] else . end
  | .[$k] |= setpath($p[1:]; $v)
  end;
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

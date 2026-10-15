# Decodifica presets (saida do extract_presets.awk) em texto legivel.
# Uso: awk -f decode_preset.awk data/algos.tsv data/presets_after.txt            (todos)
#      awk -v only=212 -f decode_preset.awk data/algos.tsv data/presets_after.txt (um preset)
#      awk -v check=1 -f decode_preset.awk ...                                    (so estatistica)
BEGIN { FS = "[ \t]"; split("FX1 FX2 AMP NR CAB EQ FX3 DLY RVB", MOD, " ") }
FNR == NR { split($0, f, "\t"); NAME[f[1], f[2]] = f[3]; next }
function b(o) { return strtonum("0x" substr(S, o * 2 + 1, 2)) }
function u16(o) { return b(o) * 256 + b(o + 1) }
function code7(o) { return b(o) * 128 + b(o + 1) }
function ascii(o, n,   s, i, c) { s = ""; for (i = 0; i < n; i++) { c = b(o + i); if (c) s = s sprintf("%c", c) } return s }
{
    idx = $1; S = $2
    if (only != "" && idx != only) next
    if (!check) printf "== idx %d (banco %d slot %d) \"%s\" volume=%d tempo=%d volpos=%s\n", idx, b(0), b(1), ascii(4, 16), u16(2), u16(0x18A), b(0x1AC) ? "Pre" : "Post"
    for (k = 0; k < 9; k++) {
        o = 0x5F + 33 * k; sw = b(o); c = code7(o + 1); m = MOD[k + 1]
        nm = ((m, c) in NAME) ? NAME[m, c] : "?"
        if (nm == "?") miss[m]++; else hit++
        if (sw > 1) badsw++
        if (check) continue
        printf "  %-3s %s %-22s [%3d] ", m, sw ? "ON " : "off", nm, c
        for (p = 0; p < 15; p++) printf "%d ", u16(o + 3 + 2 * p)
        print ""
    }
    if (check) next
    printf "  cadeia:"; for (k = 0; k < 9; k++) printf " %s", MOD[b(0x19A + 2 * k) + 1]; print ""
    printf "  quick knobs:"; for (q = 0; q < 3; q++) { o = 0x18C + 4 * q; printf " [%s alg=%d p=%d]", MOD[b(o) - 2], code7(o + 1), b(o + 3) } print ""
    printf "  EXP:"; for (r = 0; r < 8; r++) { o = 0x27 + 7 * r; if (b(o)) printf " [%s alg=%d p=%d %d..%d]", MOD[b(o)], code7(o + 1), b(o + 3), b(o + 4), b(o + 5) } print ""
}
END { if (check) { printf "algoritmos reconhecidos=%d switch>1=%d\n", hit, badsw; for (m in miss) printf "  nao reconhecidos %s: %d\n", m, miss[m] } }

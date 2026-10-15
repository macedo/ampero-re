# Extrai os presets lidos na sync (comandos 02 08 / 0E / 10) de um log do proxy.
# Saida: uma linha por preset: "<idx14> <hex de 459 bytes>"  (checksum validado e removido)
# Uso: awk -f extract_presets.awk ampero_midi_*.log > presets.txt
function hexv(s) { return strtonum("0x" s) }
$2 == "OUT" && $16 == "08" { idx = hexv($18) * 128 + hexv($19); m = 0; next }
$2 == "IN" && $16 == "0E" { for (i = 22; i < NF; i++) nib[m++] = hexv($i); next }
$2 == "IN" && $16 == "10" {
    s = 0; for (i = 0; i < m - 2; i++) s = (s + nib[i]) % 256
    if (s != nib[m - 2] * 16 + nib[m - 1]) { printf "checksum invalido idx=%d\n", idx > "/dev/stderr"; next }
    out = ""; for (i = 0; i < m - 2; i += 2) out = out sprintf("%02X", nib[i] * 16 + nib[i + 1])
    print idx, out
}

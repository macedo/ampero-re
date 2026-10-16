# Gera o ícone do app (um knob dourado sobre fundo escuro) em client/assets:
# icon.png (256 px, janela da GUI) e icon.ico (16/24/32/48/64/256 px, executável e instalador).
# Uso: powershell -ExecutionPolicy Bypass -File tools\gen_icon.ps1
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$assets = Join-Path (Split-Path -Parent $PSScriptRoot) 'client\assets'

function Draw-Icon([int]$size) {
    $bmp = New-Object System.Drawing.Bitmap $size, $size
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.Clear([System.Drawing.Color]::Transparent)
    $s = $size / 256.0

    # fundo: quadrado arredondado escuro
    $r = 56 * $s; $m = 8 * $s; $w = $size - 2 * $m
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $path.AddArc($m, $m, $r, $r, 180, 90)
    $path.AddArc($m + $w - $r, $m, $r, $r, 270, 90)
    $path.AddArc($m + $w - $r, $m + $w - $r, $r, $r, 0, 90)
    $path.AddArc($m, $m + $w - $r, $r, $r, 90, 90)
    $path.CloseFigure()
    $g.FillPath((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 30, 30, 36))), $path)

    $gold = [System.Drawing.Color]::FromArgb(255, 240, 190, 70)
    $c = $size / 2.0

    # trilha e arco de valor (270°, de 135° a 405°, valor em ~70%)
    $ar = 84 * $s
    $track = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(255, 70, 70, 80)), (18 * $s)
    $track.StartCap = 'Round'; $track.EndCap = 'Round'
    $g.DrawArc($track, $c - $ar, $c - $ar, 2 * $ar, 2 * $ar, 135, 270)
    $arc = New-Object System.Drawing.Pen $gold, (18 * $s)
    $arc.StartCap = 'Round'; $arc.EndCap = 'Round'
    $g.DrawArc($arc, $c - $ar, $c - $ar, 2 * $ar, 2 * $ar, 135, 190)

    # corpo do knob e ponteiro
    $kr = 56 * $s
    $g.FillEllipse((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 58, 58, 66))), $c - $kr, $c - $kr, 2 * $kr, 2 * $kr)
    $angle = (135 + 190) * [Math]::PI / 180
    $ptr = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(255, 245, 245, 245)), (12 * $s)
    $ptr.StartCap = 'Round'; $ptr.EndCap = 'Round'
    $g.DrawLine($ptr, $c + [Math]::Cos($angle) * 14 * $s, $c + [Math]::Sin($angle) * 14 * $s,
        $c + [Math]::Cos($angle) * 46 * $s, $c + [Math]::Sin($angle) * 46 * $s)

    $g.Dispose()
    return $bmp
}

function Png-Bytes($bmp) {
    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    return , $ms.ToArray()
}

$big = Draw-Icon 256
$big.Save((Join-Path $assets 'icon.png'), [System.Drawing.Imaging.ImageFormat]::Png)

# .ico com imagens PNG embutidas (aceito desde o Windows Vista)
$sizes = 16, 24, 32, 48, 64, 256
$images = foreach ($sz in $sizes) { , (Png-Bytes (Draw-Icon $sz)) }
$out = New-Object System.IO.MemoryStream
$bw = New-Object System.IO.BinaryWriter $out
$bw.Write([UInt16]0); $bw.Write([UInt16]1); $bw.Write([UInt16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $sz = $sizes[$i]; $len = $images[$i].Length
    $dim = if ($sz -ge 256) { 0 } else { $sz }
    $bw.Write([byte]$dim); $bw.Write([byte]$dim); $bw.Write([byte]0); $bw.Write([byte]0)
    $bw.Write([UInt16]1); $bw.Write([UInt16]32); $bw.Write([UInt32]$len); $bw.Write([UInt32]$offset)
    $offset += $len
}
foreach ($img in $images) { $bw.Write($img) }
$bw.Flush()
[System.IO.File]::WriteAllBytes((Join-Path $assets 'icon.ico'), $out.ToArray())
Write-Host "Icones gerados em $assets"

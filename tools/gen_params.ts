// Gera client/assets/params.json a partir do algorithm.xml do editor oficial.
// Uso: deno run --allow-read --allow-write tools/gen_params.ts <algorithm.xml> <saida.json>
// Formato: { "<MOD>:<codigo>": { name, params: [{ index, name, kind, min, max, display_min, display_max, default, options }] } }

const [src, out] = Deno.args;
const xml = Deno.readTextFileSync(src);

const attrs = (tag: string): Record<string, string> =>
  Object.fromEntries([...tag.matchAll(/([A-Za-z]+)\s*=\s*"([^"]*)"/g)].map((m) => [m[1], m[2].trim()]));

const num = (v: string | undefined) => (v === undefined || v === "" ? undefined : Number(v));

const result: Record<string, unknown> = {};
for (const m of xml.matchAll(/<Algorithm\b([^>]*)>([\s\S]*?)<\/Algorithm>/g)) {
  const a = attrs(m[1]);
  const code = Number(a.BaseIndex) + Number(a.ChildIndex);
  const params = [];
  for (const e of m[2].matchAll(/<(Knob|Combox|Switch|Slider|TextEditor)\b([^>]*?)(\/>|>([\s\S]*?)<\/\1>)/g)) {
    const kind = e[1].toLowerCase();
    const p = attrs(e[2]);
    const options = kind === "combox" ? [...(e[4] ?? "").matchAll(/<Menu\b([^>]*)\/>/g)].map((o) => attrs(o[1]).Name) : [];
    const dmin = num(p.DisplayMin), dmax = num(p.DisplayMax);
    let min = num(p.Min) ?? dmin ?? 0, max = num(p.Max) ?? dmax ?? 1;
    if (kind === "combox") [min, max] = [0, Math.max(options.length - 1, 0)];
    if (kind === "switch") [min, max] = [0, 1];
    params.push({
      index: Number(p.ID) - 1,
      name: p.Name,
      kind,
      min,
      max,
      display_min: kind === "combox" || kind === "switch" ? min : dmin ?? min,
      display_max: kind === "combox" || kind === "switch" ? max : dmax ?? max,
      default: num(p.DefaultValue) ?? 0,
      options,
    });
  }
  params.sort((x, y) => x.index - y.index);
  result[`${a.Module}:${code}`] = { name: a.Name, params };
}

Deno.writeTextFileSync(out, JSON.stringify(result));
console.log(`${Object.keys(result).length} algoritmos, ${Object.values(result).reduce((n, v: any) => n + v.params.length, 0)} parâmetros`);

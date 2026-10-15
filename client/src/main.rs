use ampero::device::{self, Device, all_preset_indices};
use ampero::preset::{MODULES, parse_label};
use ampero::{logfile, protocol};
use anyhow::{Context, Result, bail};
use std::time::Instant;

const USAGE: &str = "\
ampero — cliente multiplataforma para o Hotone Ampero

USO:
  ampero ports                         lista portas MIDI
  ampero sync [--json ARQ]             lê nomes + todos os presets e mede o tempo
  ampero show P29-1                    lê um preset do banco de usuário
  ampero current                       lê o buffer de edição (preset atual)
  ampero diff                          compara o buffer de edição com o preset salvo (byte a byte)
  ampero select P29-1                  seleciona preset (CC0 + Program Change)
  ampero set MOD PARAM VALOR           altera parâmetro no buffer de edição (ex.: set AMP 0 20)
  ampero decode-log LOG [--json ARQ]   decodifica presets de um log do proxy winmm (offline)
  ampero monitor [SEGUNDOS]           mostra tudo que o aparelho envia (padrão: 60 s)

OPÇÕES:
  --port NOME   trecho do nome da porta MIDI (padrão: Ampero)
  --bank N      banco para show/select (padrão: 1 = User Patches)";

struct Args {
    cmd: String,
    pos: Vec<String>,
    port: String,
    bank: u8,
    json: Option<String>,
}

fn parse_args() -> Result<Args> {
    let mut it = std::env::args().skip(1);
    let mut a = Args {
        cmd: String::new(),
        pos: vec![],
        port: "Ampero".into(),
        bank: 1,
        json: None,
    };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--port" => a.port = it.next().context("--port precisa de valor")?,
            "--bank" => a.bank = it.next().context("--bank precisa de valor")?.parse()?,
            "--json" => a.json = Some(it.next().context("--json precisa de valor")?),
            "-h" | "--help" => a.cmd = "help".into(),
            _ if a.cmd.is_empty() => a.cmd = arg,
            _ => a.pos.push(arg),
        }
    }
    Ok(a)
}

fn preset_index(bank: u8, label: &str) -> Result<(u16, u8)> {
    let slot = parse_label(label)
        .with_context(|| format!("rótulo inválido: {label} (use P01-1 … P33-3)"))?;
    Ok((bank as u16 * 128 + slot as u16, slot))
}

fn write_json(path: &Option<String>, value: &impl serde::Serialize) -> Result<()> {
    if let Some(path) = path {
        std::fs::write(path, serde_json::to_string_pretty(value)?)?;
        eprintln!("salvo em {path}");
    }
    Ok(())
}

fn main() -> Result<()> {
    let a = parse_args()?;
    match a.cmd.as_str() {
        "ports" => {
            let (ins, outs) = device::list_ports()?;
            println!("Entradas:");
            ins.iter().for_each(|n| println!("  {n}"));
            println!("Saídas:");
            outs.iter().for_each(|n| println!("  {n}"));
        }
        "sync" => {
            let mut dev = Device::open(&a.port)?;
            let t0 = Instant::now();
            dev.handshake(0)?;
            dev.global_state()?;
            let clones: Vec<_> = (0..11).map(|i| dev.name(0x05, i)).collect::<Result<_>>()?;
            let irs: Vec<_> = (0..20).map(|i| dev.name(0x07, i)).collect::<Result<_>>()?;
            let t_names = t0.elapsed();
            let mut presets = Vec::with_capacity(198);
            for idx in all_preset_indices() {
                presets.push(dev.read_preset(idx)?);
            }
            let total = t0.elapsed();
            for pr in &presets {
                println!(
                    "[{:3}] banco {} {} {}",
                    pr.index, pr.bank, pr.label, pr.name
                );
            }
            println!(
                "Capturas: {:?}",
                clones.iter().flatten().collect::<Vec<_>>()
            );
            println!("IRs:      {:?}", irs.iter().flatten().collect::<Vec<_>>());
            println!(
                "\n{} presets em {:.2} s (nomes {:.0} ms) — {} pedidos, {:.2} ms/pedido, {} notificações ignoradas",
                presets.len(),
                total.as_secs_f64(),
                t_names.as_secs_f64() * 1e3,
                dev.round_trips,
                total.as_secs_f64() * 1e3 / dev.round_trips as f64,
                dev.unsolicited
            );
            write_json(
                &a.json,
                &serde_json::json!({ "clones": clones, "user_irs": irs, "presets": presets }),
            )?;
        }
        "show" => {
            let label = a.pos.first().context("uso: ampero show P29-1")?;
            let (idx, _) = preset_index(a.bank, label)?;
            let mut dev = Device::open(&a.port)?;
            let t0 = Instant::now();
            let pr = dev.read_preset(idx)?;
            print!("{}", pr.summary());
            eprintln!("lido em {:.1} ms", t0.elapsed().as_secs_f64() * 1e3);
            write_json(&a.json, &pr)?;
        }
        "current" => {
            let mut dev = Device::open(&a.port)?;
            let pr = dev.read_edit_buffer()?;
            print!("{}", pr.summary());
            write_json(&a.json, &pr)?;
        }
        "diff" => {
            let mut dev = Device::open(&a.port)?;
            let cur = dev.read_edit_buffer()?;
            let saved = dev.read_preset(cur.index)?;
            println!(
                "buffer de edição vs salvo: banco {} {} \"{}\"",
                cur.bank, cur.label, cur.name
            );
            for (o, (b, s)) in cur.raw.iter().zip(&saved.raw).enumerate() {
                if b != s {
                    println!("  0x{o:03X}: salvo {s:02X} -> buffer {b:02X}");
                }
            }
        }
        "select" => {
            let label = a.pos.first().context("uso: ampero select P29-1")?;
            let (_, slot) = preset_index(a.bank, label)?;
            let mut dev = Device::open(&a.port)?;
            for msg in protocol::select_preset(a.bank, slot) {
                dev.send(&msg)?;
            }
            println!("selecionado banco {} {}", a.bank, label);
        }
        "set" => {
            let [module, param, value] = a.pos.as_slice() else {
                bail!("uso: ampero set MOD PARAM VALOR")
            };
            let k = MODULES
                .iter()
                .position(|m| m.eq_ignore_ascii_case(module))
                .with_context(|| {
                    format!("módulo inválido: {module} (use {})", MODULES.join(", "))
                })?;
            let (param, value): (u8, u16) = (param.parse()?, value.parse()?);
            let mut dev = Device::open(&a.port)?;
            let current = dev.read_edit_buffer()?;
            let block = &current.blocks[k];
            dev.send(&protocol::set_param(
                k as u8 + 1,
                block.algorithm,
                param,
                value,
            ))?;
            println!(
                "{} {} [{}] param {param}: {} -> {value}",
                block.module,
                block.algorithm_name.as_deref().unwrap_or("?"),
                current.label,
                block.params.get(param as usize).copied().unwrap_or(0)
            );
        }
        "monitor" => {
            let secs: u64 = a.pos.first().map(|s| s.parse()).transpose()?.unwrap_or(60);
            let mut dev = Device::open(&a.port)?;
            let t0 = Instant::now();
            let end = t0 + std::time::Duration::from_secs(secs);
            eprintln!("monitorando por {secs} s… mexa na pedaleira");
            while let Some(left) = end.checked_duration_since(Instant::now()) {
                if let Some(msg) = dev.recv(left) {
                    println!(
                        "{:9.3} {:3}: {}",
                        t0.elapsed().as_secs_f64(),
                        msg.len(),
                        device::hex(&msg)
                    );
                }
            }
        }
        "decode-log" => {
            let path = a.pos.first().context("uso: ampero decode-log LOG")?;
            let presets = logfile::presets_from_log(&std::fs::read_to_string(path)?)?;
            for pr in &presets {
                print!("{}", pr.summary());
            }
            eprintln!("{} presets decodificados", presets.len());
            write_json(&a.json, &presets)?;
        }
        _ => println!("{USAGE}"),
    }
    Ok(())
}

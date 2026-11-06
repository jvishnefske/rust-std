use std::io::{self, BufRead};

fn main() -> Result<(), String> {
    let mut total = 0i64;
    for line in io::stdin().lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        total += line.trim().parse::<i64>().map_err(|e| format!("bad number {line:?}: {e}"))?;
    }
    println!("total: {total}");
    Ok(())
}

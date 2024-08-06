use colored::Colorize;
use spackle::core::config::Config;

pub fn run(config: &Config) {
    // Print slot info
    println!("🕳️  {}", "slots".truecolor(140, 200, 255).bold());

    (&config.slots).into_iter().for_each(|slot| {
        println!("{}\n", slot);
    });

    // Print hook info
    println!("🪝  {}", "hooks".truecolor(140, 200, 255).bold());

    (&config.hooks).into_iter().for_each(|hook| {
        println!("{}\n", hook);
    });
}

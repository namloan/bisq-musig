pub fn setup() {
    use std::process::Command;

    println!("Starting the setup...");
    //
    // // Step 1: Run the curl command to download and install Nigiri
    // let curl_output = Command::new("sh")
    //     .arg("-c")
    //     .arg("curl https://getnigiri.vulpem.com | bash")
    //     .output();
    //
    // match curl_output {
    //     Ok(output) => {
    //         if output.status.success() {
    //             println!("Successfully ran the curl command.");
    //         } else {
    //             eprintln!(
    //                 "Failed to run the curl command. Error: {}",
    //                 String::from_utf8_lossy(&output.stderr)
    //             );
    //             std::process::exit(1);
    //         }
    //     }
    //     Err(e) => {
    //         eprintln!("Error while running the curl command: {}", e);
    //         std::process::exit(1);
    //     }
    // }

    // Step 2: Run 'nigiri start' to start Nigiri
    let nigiri_output = Command::new("nigiri").arg("start").output();

    match nigiri_output {
        Ok(output) => {
            if output.status.success() {
                println!("Nigiri started successfully.");
            } else {
                eprintln!(
                    "Failed to start Nigiri. Error: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Error while starting Nigiri Did you install Nigiri?: {}", e);
            std::process::exit(1);
        }
    }

    println!("Setup completed successfully.");
}

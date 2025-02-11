pub fn setup() {
    use std::process::Command;
        
    println!("Starting the setup...");

    // Check if Docker is running
    let docker_check = Command::new("docker")
        .arg("info")
        .output();

    match docker_check {
        Ok(output) => {
            if !output.status.success() {
                eprintln!("Docker is not running. Please start Docker first.");
                std::process::exit(1);
            }
        }
        Err(_) => {
            eprintln!("Docker is not installed or not in PATH. Please install Docker first.");
            std::process::exit(1);
        }
    }

    // Run 'nigiri start' to start Nigiri
    let nigiri_output = Command::new("nigiri")
        .arg("start")
        .output();

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
            eprintln!("Error while starting Nigiri: {}", e);
            std::process::exit(1);
        }
    }

    println!("Setup completed successfully.");
}


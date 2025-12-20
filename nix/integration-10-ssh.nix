{
  pkgs,
  yeet,
}:
let
  testPrivateKey = builtins.readFile ./yeet_ssh_sut_key;
  testPublicKey = builtins.readFile ./yeet_ssh_sut_key.pub;

  yeetModule = import ./module.nix;
in
pkgs.testers.runNixOSTest {
  # This is me just playing with setting up a two node vm setup so I can prep for having two daemons talk to each other.
  name = "yeet-ssh-test";

  nodes = {
    node1 =
      { pkgs, ... }:
      {
        imports = [ yeetModule ];

        environment.systemPackages = [ yeet ];

        services.yeet = {
          enable = true;
          package = yeet;
        };

        services.openssh = {
          enable = true;
          settings = {
            PermitRootLogin = "yes";
            PasswordAuthentication = false;
            PubkeyAuthentication = true;
          };
        };

        users.users.testuser = {
          isNormalUser = true;
          extraGroups = [ "wheel" ];
          openssh.authorizedKeys.keys = [ testPublicKey ];
        };

        security.sudo.wheelNeedsPassword = false;

        networking.firewall.allowedTCPPorts = [ 22 ];
      };

    node2 =
      { pkgs, ... }:
      {
        imports = [ yeetModule ];

        environment.systemPackages = [ yeet ];

        services.yeet = {
          enable = true;
          package = yeet;
        };

        services.openssh = {
          enable = true;
          settings = {
            PermitRootLogin = "yes";
            PasswordAuthentication = false;
            PubkeyAuthentication = true;
          };
        };

        users.users.testuser = {
          isNormalUser = true;
          extraGroups = [ "wheel" ];
          openssh.authorizedKeys.keys = [ testPublicKey ];
        };

        security.sudo.wheelNeedsPassword = false;

        networking.firewall.allowedTCPPorts = [ 22 ];
      };
  };

  testScript = ''
    start_all()

    # Wait for both nodes to be ready
    node1.wait_for_unit("multi-user.target")
    node2.wait_for_unit("multi-user.target")

    # Wait for yeet daemon to be available on both nodes
    node1.wait_for_unit("yeet.service")
    node2.wait_for_unit("yeet.service")

    # Verify yeet daemon is active
    node1.succeed("systemctl is-active yeet.service")
    node2.succeed("systemctl is-active yeet.service")

    # Check for daemon startup failures or crashes on both nodes
    node1.fail("journalctl -u yeet.service | grep -Ei '(failed|panic)'")
    node2.fail("journalctl -u yeet.service | grep -Ei '(failed|panic)'")

    # Simple check for restarts. This honestly is likely a thing we can make as a library spiel
    node1_restarts = node1.succeed("systemctl show yeet.service -p NRestarts --value").strip()
    node2_restarts = node2.succeed("systemctl show yeet.service -p NRestarts --value").strip()
    assert node1_restarts == "0", f"node1 yeet service restarted {node1_restarts} times"
    assert node2_restarts == "0", f"node2 yeet service restarted {node2_restarts} times"

    # We'll need ssh to be up for anything to work
    node1.wait_for_unit("sshd.service")
    node2.wait_for_unit("sshd.service")

    # As well as a network to talk over obvs
    node1.wait_for_unit("network.target")
    node2.wait_for_unit("network.target")

    # Gotta be a better way to get ip addresses than this hack
    node1_ip = node1.succeed("ip -4 addr show dev eth1 | grep -oP '(?<=inet\\s)\\d+(\\.\\d+){3}'").strip()
    node2_ip = node2.succeed("ip -4 addr show dev eth1 | grep -oP '(?<=inet\\s)\\d+(\\.\\d+){3}'").strip()

    print(f"node1 IP: {node1_ip}")
    print(f"node2 IP: {node2_ip}")

    # Verify yeet binary is present
    node1.succeed("yeet -V")
    node2.succeed("yeet -V")

    # Simple smoke test look for the domain socket
    node1.succeed("test -S /root/.cache/yeet/local.uds")
    node2.succeed("test -S /root/.cache/yeet/local.uds")

    # Setup SSH directory and install private key on both nodes for testuser
    node1.succeed("mkdir -p /home/testuser/.ssh")
    node1.succeed("chmod 700 /home/testuser/.ssh")
    node1.copy_from_host("${./yeet_ssh_sut_key}", "/home/testuser/.ssh/id_ed25519")
    node1.succeed("chmod 600 /home/testuser/.ssh/id_ed25519")
    node1.succeed("chown -R testuser:users /home/testuser/.ssh")

    node2.succeed("mkdir -p /home/testuser/.ssh")
    node2.succeed("chmod 700 /home/testuser/.ssh")
    node2.copy_from_host("${./yeet_ssh_sut_key}", "/home/testuser/.ssh/id_ed25519")
    node2.succeed("chmod 600 /home/testuser/.ssh/id_ed25519")
    node2.succeed("chown -R testuser:users /home/testuser/.ssh")

    # Be sure we can ssh between node 1 and 2
    node1.succeed(
        f"sudo -u testuser ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null testuser@{node2_ip} 'echo hello from node2'"
    )

    # And back
    node2.succeed(
        f"sudo -u testuser ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null testuser@{node1_ip} 'echo hello from node1'"
    )

    # See if we can run yeet from node 1 on node 2
    node1.succeed(
        f"sudo -u testuser ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null testuser@{node2_ip} 'yeet -V'"
    )

    # ditto reverse
    node2.succeed(
        f"sudo -u testuser ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null testuser@{node1_ip} 'yeet -V'"
    )

    # This is more future looking
    node1.succeed("systemctl is-active yeet.service")
    node2.succeed("systemctl is-active yeet.service")

    # Look for nothing to have restarted yet
    node1_restarts_final = node1.succeed("systemctl show yeet.service -p NRestarts --value").strip()
    node2_restarts_final = node2.succeed("systemctl show yeet.service -p NRestarts --value").strip()
    assert node1_restarts_final == "0", f"node1 daemon crashed during test ({node1_restarts_final} restarts)"
    assert node2_restarts_final == "0", f"node2 daemon crashed during test ({node2_restarts_final} restarts)"

    # Test heartbeat from node1 to node2 over ssh works and back
    # I had some wip stuff I committed ignore this for now in this test
    # node1.succeed(f"yeet sut heartbeat {node2_ip}")
    # node2.succeed(f"yeet sut heartbeat {node1_ip}")

    print("simple two node smoketest passed, hopefully all is well")
  '';
}

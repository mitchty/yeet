{ pkgs, yeet-dev }:
let
  yeetModule = import ./module.nix;
in
pkgs.testers.runNixOSTest {
  name = "yeet-integration-simple";

  nodes.machine =
    { pkgs, ... }:
    {
      imports = [ yeetModule ];

      environment.systemPackages = [ yeet-dev ];

      services.yeet = {
        enable = true;
        package = yeet-dev;
      };
    };

  testScript = ''
    start_all()
    # Basic simple tests that should always work
    machine.succeed("yeet -V")
    machine.succeed("yeet --version")

    machine.wait_for_unit("multi-user.target")

    # Service tests - verify yeet daemon... daemons
    machine.wait_for_unit("yeet.service")
    machine.succeed("systemctl is-active yeet.service")
    machine.succeed("systemctl status yeet.service")

    # Check for any failures in journal output
    machine.fail("journalctl -u yeet.service | grep -Ei '(failed|error|panic)'")

    # Validate that we don't see that restart counts increase, if so somethings broken no need to test anything else
    restart_count = machine.succeed("systemctl show yeet.service -p NRestarts --value").strip()
    assert restart_count == "0", f"Service has restarted {restart_count} times (expected 0)"

    # Be sure yeets in an active state
    state = machine.succeed("systemctl show yeet.service -p ActiveState --value").strip()
    assert state == "active", f"Service state is {state} (expected active)"

    # Check that the daemon created its unix domain socket
    # TODO: XDG setup for root isn't a sane thing, I should make the dir the sockets in configurable in this instance
    machine.wait_until_succeeds("test -S /root/.cache/yeet/local.uds")

    # Simple grpc query (soon) that will send a heartbeat message? For now whatever
    machine.succeed("yeet grpc --socketonly")

    # Let the service run for a bit to ensure it isn't crashing over a few seconds and we didn't miss crashes/issues due to timing
    import time
    time.sleep(3)

    machine.succeed("systemctl is-active yeet.service")

    # Be sure our restart count hasn't incremented
    restart_count_after = machine.succeed("systemctl show yeet.service -p NRestarts --value").strip()
    assert restart_count_after == "0", f"Service crashed and restarted {restart_count_after} times"

    # Ensure systemd can restart things ok
    machine.succeed("systemctl restart yeet.service")
    machine.wait_for_unit("yeet.service")
    machine.succeed("systemctl is-active yeet.service")

    # Be sure our sockets present, better logic to come
    machine.succeed("test -S /root/.cache/yeet/local.uds")

    # Give a bit of time again to be sure all is ok still
    time.sleep(2)
    machine.succeed("systemctl is-active yeet.service")

    # Cross fingies that we aren't crashing after a manual restart
    restart_count_manual = machine.succeed("systemctl show yeet.service -p NRestarts --value").strip()
    assert restart_count_manual == "0", f"Unexpected automatic restarts after manual restart: {restart_count_manual}"

    # Final check: no errors in journal after restart
    machine.fail("journalctl -u yeet.service --since '10 seconds ago' | grep -Ei '(error|panic)'")

    print("Simple integration test passed - binary works, daemon starts, seemingly runs stable, and restarts successfully, might not suck!")
  '';
}

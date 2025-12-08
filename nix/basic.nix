{ pkgs, yeet }:
pkgs.testers.runNixOSTest {
  name = "yeet-basic-test";

  nodes.machine =
    { pkgs, ... }:
    {
      environment.systemPackages = [ yeet ];
    };

  # Lets start with just making sure yeet -V works. Things will get complext
  # from here on out.
  testScript = ''
    start_all()
    machine.wait_for_unit("multi-user.target")

    machine.succeed("yeet -V")
  '';
}

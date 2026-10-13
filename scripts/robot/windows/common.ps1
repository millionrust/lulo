# Shared set-up for the robot scripts: compiles RobotLib.cs once per run.
Add-Type -AssemblyName System.Drawing, System.Windows.Forms
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not ('LuloRobot.Robot' -as [type])) {
    Add-Type -Path (Join-Path $here 'RobotLib.cs') -ReferencedAssemblies @(
        'System.Drawing', 'UIAutomationClient', 'UIAutomationTypes', 'WindowsBase'
    )
}
$R = [LuloRobot.Robot]

# Virtual-key codes the robot uses.
$VK = @{
    Back = 0x08; Tab = 0x09; Return = 0x0D; Shift = 0x10; Ctrl = 0x11; Alt = 0x12; Esc = 0x1B; Space = 0x20
    PgUp = 0x21; PgDn = 0x22; End = 0x23; Home = 0x24; Left = 0x25; Up = 0x26; Right = 0x27; Down = 0x28
    Delete = 0x2E; Win = 0x5B; F1 = 0x70; F4 = 0x73; Comma = 0xBC; Plus = 0xBB; Minus = 0xBD
}

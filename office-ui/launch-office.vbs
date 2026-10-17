Set WshShell = CreateObject("WScript.Shell")
WshShell.CurrentDirectory = "C:\Users\chrom\Desktop\Marker\atlas-studio\office-ui"
WshShell.Run """C:\Users\chrom\Desktop\Marker\atlas-studio\office-ui\node_modules\electron\dist\electron.exe"" .", 1, False
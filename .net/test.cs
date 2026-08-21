using System;
using System.Reflection;
using ImageMagick;

class P {
    static void TestMagick() {
        foreach (var m in typeof(MagickImage).GetMethods()) {
            if (m.Name.ToLower().Contains("page") || m.Name.ToLower().Contains("trim") || m.Name.ToLower().Contains("box")) {
                Console.WriteLine(m.Name);
            }
        }
    }
}

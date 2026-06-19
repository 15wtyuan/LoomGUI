// 暴露 LoomGUI.Bindings 的 internal（csbindgen 生成的 Native / StageHandle）给 LoomGUI.Runtime。
// csbindgen 默认生成 internal 类型；LoomStage（LoomGUI.Runtime 程序集）需跨程序集调用 Native.*。
using System.Runtime.CompilerServices;

[assembly: InternalsVisibleTo("LoomGUI.Runtime")]

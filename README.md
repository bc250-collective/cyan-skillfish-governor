# Cyan Skillfish GPU governor

> **IMPORTANT**
> This build requires BIOS/UEFI **Robin 3.00** to function correctly.
> It does **not** work correctly with BIOS/UEFI **Robin 5.00**.
>
> Config location and sturcture is different from the original `cyan-skillfish-governor`:
> - It uses: `/etc/cyan-skillfish-governor-smu-plus/config.toml`
> - Config structure changes are described at the end of readme.

Huge thanks to:
- Original developer @Magnap
- SMU python to rust implementation, thermal throttling functionality to @filippor

GPU governor for the AMD Cyan Skillfish APU.
Continously maintains a target frequency, and adjusts the actual GPU frequency when the deviation is too great.
If the CPU is continously busy for too long, ramps up the target frequency rapidly.

this version sets voltage/frequency/memory controller profile using SMU firmware commands.

Takes a TOML config file path as its only argument.
Keys are:
* `timing`
  * `intervals`: in µs
    * `sample`: how often to sample GPU load
      (it's a single bit, so needs to be sampled more often than you'd think)
    * `adjust`: how often to consider adjusting the frequency
  * `burst-samples`: while the GPU has been busy for this many samples in a row,
    enter "burst mode", increasing the frequency at the `timing.ramp_rates.burst` rate.
    Set to 0 to disable burst mode.
  * `down-events`: number of event below `load-target.low` to step down
  * `ramp_rates`: how quickly to increase/decrease GPU frequency, in MHz/ms
    * `normal`: ramp rate for normal adjustments
    * `burst`: ramp rate in burst mode
* `frequency-thresholds`: in MHz
  * `adjust`: how large a proposed adjustment must be to actually be carried out
* `load-target`: as a fraction
  * `upper`: GPU load above which target frequency is increased
  * `lower`: GPU load below which target frequency is decreased
* `cpu-load-target`: as a fraction
  * `upper`: max per-core CPU load above which `perf_profile` is temporarily forced to `3`
  * `lower`: max per-core CPU load below which `perf_profile` returns to safe-point value
  * if undefined, falls back to `load-target`
* `temmperature` in °C
  * `throttling` if temperature is greather  start reducing max frequency
  * `throttling_recovery` if temperaure is lower restore max frequency
* `safe-points`: known safe/stable power points, array of tables with keys:
  * `frequency`: GPU frequency in MHz
  * `voltage`: GPU supply voltage in mV
  * `perf_profile`: SMU performance profile index for memory controller / Infinity Fabric
    (0-3, default: 3)
    * `3`: highest memory controller / IF performance (default if omitted)
    * `1`: recommended low-power profile for lowest idle point
    * `0`: lowest profile, usually no practical power benefit vs `1`


Frequency scaling remains GPU-load driven, while perf profile has a separate CPU override path:
if CPU load exceeds `cpu-load-target.upper`, profile `3` is forced; when CPU load drops below
`cpu-load-target.lower`, it returns to the current safe-point profile.  
CPU load for this override is measured as max per-core utilization and sampled 2 times per second.
Typical setup is to use `perf_profile = 1` only for the lowest idle safe-point, and `perf_profile = 3` for all other points.

Example:

```toml
[[safe-points]]
frequency = 1000
voltage = 800
perf_profile = 1

[[safe-points]]
frequency = 1175
voltage = 850
perf_profile = 3
```

See also `default-config.toml`.

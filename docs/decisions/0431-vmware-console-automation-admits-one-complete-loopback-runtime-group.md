# D-431 VMware console automation admits one complete loopback runtime group

Status: Accepted, 2026-10-23. Amends D-365; advances PW-579.

**Context.** The authorized VMware input rounds need a host console path
without guest software. On this host, `vmrun` guest typing refuses and the
Workstation MKS commands do not change the observed firmware menu. An
isolated diagnostic boot with Workstation's built-in VNC console accepts an
RFB 3.8 Down key and visibly moves that menu. The observed listener belongs
to the sole `vmware-vmx` process and binds only `127.0.0.1:5997`; the exact
one-machine `vmrun` list identifies the probe, and stopping it removes the
listener. This establishes a console transport, not a kernel qualification.

**Decision.** The runtime VMX may omit VNC entirely or carry exactly this
complete group, with the original spelling and values:

```text
RemoteDisplay.vnc.enabled = "TRUE"
RemoteDisplay.vnc.ip = "127.0.0.1"
RemoteDisplay.vnc.port = "5997"
```

Partial groups cannot inherit an unreviewed default. Disabled or alternate
enable values, wildcard or other addresses, hostnames, different ports and
alternate decimal spellings refuse. Every other `RemoteDisplay` addition
continues to refuse, including password and key-map settings. The source
template and generation receipt do not change: every generated assignment
must still be present unchanged, while the captured runtime VMX and serial
transcript remain exact evidence. No assignment is removed before hashing,
and VNC grants no CPU-mask, device, transcript or receipt exception.

The operator adds the group only to the stopped generated runtime machine,
then verifies the exact one-machine identity and actual loopback listener
ownership before connecting locally. The console has no VNC authentication;
it is bounded to the existing host authority, not isolated from other local
processes. Input follows the real prompts, qualification still observes the
running machine, and stopping it must remove the listener. This is an
optional input transport, not a background service or a new qualification
verdict. Fresh self-test and shipped machine evidence remain required.

**Proof.** The focused config witness accepts the absent group and the exact
complete group alongside ordinary Workstation identity additions. It
rejects all six partial subsets, alternate values, unreviewed keys,
case-colliding duplicates and source-setting changes with VNC enabled.
Retained mutations 2,327 through 2,337 weaken completeness, optionality,
canonical enable values, address and port constraints, the unknown-key
boundary and generated-assignment binding. On 2026-10-23 all six config
tests passed, each of those eleven mutations returned exit 101 with the
exact witness marked FAILED, and the pristine witness passed before and
after the series. Every mutated source was restored byte for byte and its
SHA-256 rechecked. Ordinary CI and current-source machine qualification
remain separate requirements.

**Alternatives.** An unrestricted VNC port range or address exception would
admit configurations the probe did not establish. Putting the console in the
source template would require it for every generated machine. Rewriting the
receipt or filtering VNC keys from captured evidence would hide execution
inputs. Requiring guest-agent typing cannot serve this freestanding kernel.

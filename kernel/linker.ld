/* Linker script for the higher-half x86_64 kernel, loaded by Limine. */

OUTPUT_FORMAT(elf64-x86-64)
ENTRY(kmain)

PHDRS
{
    text    PT_LOAD;
    rodata  PT_LOAD;
    data    PT_LOAD;
}

SECTIONS
{
    /* Top 2 GiB of the address space, as the kernel code model requires. */
    . = 0xffffffff80000000;

    .text : {
        *(.text .text.*)
    } :text

    . = ALIGN(CONSTANT(MAXPAGESIZE));

    .rodata : {
        *(.rodata .rodata.*)
    } :rodata

    . = ALIGN(CONSTANT(MAXPAGESIZE));

    /* Limine scans between the markers for request structures and writes its
       answers back into them, so this is loaded and writable. Its own output
       section rather than folded into .data, for two reasons: KEEP() states
       that the `#[used]` statics `wost::entry!` emits must survive section
       garbage collection whatever a future build turns on (D-21), and a
       section with a name is one `cargo xtask image` can check is there. A
       request block that silently failed to link is a kernel reading
       responses nobody filled in. */
    .requests : {
        KEEP(*(.requests_start_marker))
        KEEP(*(.requests))
        KEEP(*(.requests_end_marker))
    } :data

    .data : {
        *(.data .data.*)

        /* Named, not left to the linker's orphan placement. A static build
           should produce neither of these, but "should" is how an orphan ends
           up after .bss — which forces the whole of .bss to be written into
           the file as zeros, because a segment cannot go back to NOBITS once
           something loadable follows it. */
        *(.got .got.plt)
        *(.data.rel.ro .data.rel.ro.*)
    } :data

    /* Last in the segment and NOLOAD, so it costs nothing in the image and
       the bootloader zeroes it. `cargo xtask image` checks that it stayed
       that way, since the failure is silent bloat rather than a link error. */
    .bss (NOLOAD) : {
        *(.bss .bss.*)
        *(COMMON)
    } :data

    /DISCARD/ : {
        *(.eh_frame*)
        *(.note .note.*)
        *(.comment)
    }
}

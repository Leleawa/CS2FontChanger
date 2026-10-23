//! Fontconfig generation. Font selection happens before matching; scaling happens after it.

use crate::font::{xml_escape, FontSelection};

pub struct FontConfigs {
    pub local: String,
    pub global: String,
}

pub fn generate(fonts: &FontSelection, ui_scale: f64) -> FontConfigs {
    let regular = fonts.regular.as_ref().expect("validated regular font");
    let safe_font_name = xml_escape(&regular.family);
    let safe_bold_name = xml_escape(&fonts.bold.as_ref().unwrap_or(regular).family);
    let hints = weight_hints();
    let local = format!(
        r#"<?xml version='1.0'?>
<!DOCTYPE fontconfig SYSTEM 'fonts.dtd'>
<fontconfig>

	<dir prefix="default">../../csgo/panorama/fonts</dir>
	<dir>WINDOWSFONTDIR</dir>
	<dir>~/.fonts</dir>
	<dir>/usr/share/fonts</dir>
	<dir>/usr/local/share/fonts</dir>
	<dir prefix="xdg">fonts</dir>

	<fontpattern>Arial</fontpattern>
	<fontpattern>.uifont</fontpattern>
	<fontpattern>notosans</fontpattern>
	<fontpattern>notoserif</fontpattern>
	<fontpattern>notomono-regular</fontpattern>
	<fontpattern>{safe_font_name}</fontpattern>
	<fontpattern>{safe_bold_name}</fontpattern>
	<fontpattern>.ttf</fontpattern>
	<fontpattern>.otf</fontpattern>
	<fontpattern>FONTFILENAME</fontpattern>
	
	<cachedir>WINDOWSTEMPDIR_FONTCONFIG_CACHE</cachedir>
	<cachedir>~/.fontconfig</cachedir>

	<!-- Vietnamese language support -->
	<match>
		<test name="lang">
			<string>vi-vn</string>
		</test>
		<test name="family" compare="contains">
			<string>Stratum2</string>
		</test>
		<test qual="all" name="family" compare="not_contains">
			<string>TF</string>
		</test>
		<test qual="all" name="family" compare="not_contains">
			<string>Mono</string>
		</test>
		<test qual="all" name="family" compare="not_contains">
			<string>ForceStratum2</string>
		</test>
		<edit name="weight" mode="assign">
			<if>
				<contains>
					<name>family</name>
					<string>Stratum2 Black</string>
				</contains>
				<int>210</int>
				<name>weight</name>
			</if>
		</edit>
		<edit name="slant" mode="assign">
			<if>
				<contains>
					<name>family</name>
					<string>Italic</string>
				</contains>
				<int>100</int>
				<name>slant</name>
			</if>
		</edit>
		<edit name="pixelsize" mode="assign">
			<if>
				<or>
					<contains>
						<name>family</name>
						<string>Condensed</string>
					</contains>
					<less_eq>
						<name>width</name>
						<int>75</int>
					</less_eq>
				</or>
				<times>
					<name>pixelsize</name>
					<double>0.7</double>
				</times>
				<times>
					<name>pixelsize</name>
					<double>0.9</double>
				</times>
			</if>
		</edit>
		<edit name="family" mode="assign" binding="same">
			<string>notosans</string>
		</edit>
	</match>

	<selectfont> 
		<rejectfont> 
			<pattern> 
				<patelt name="fontformat" > 
					<string>Type 1</string> 
				</patelt> 
			</pattern> 
		</rejectfont> 
	</selectfont> 

	<match target="font" >
		<edit name="embeddedbitmap" mode="assign">
			<bool>false</bool>
		</edit>
	</match>

	<match target="pattern" >
		<edit name="prefer_outline" mode="assign">
			<bool>true</bool>
		</edit>
	</match>

	<match target="pattern" >
		<edit name="do_substitutions" mode="assign">
			<bool>true</bool>
		</edit>
	</match>

	<match target="font">
		<edit name="force_autohint" mode="assign">
			<bool>false</bool>
		</edit>
	</match>

	<include>../../../core/panorama/fonts/conf.d</include>

</fontconfig>"#
    );
    // Capture names such as Stratum2 Bold before the legacy Vietnamese alias edits them.
    let local = local.replacen(
        "\t<!-- Vietnamese language support -->",
        &format!("{hints}\n\t<!-- Vietnamese language support -->"),
        1,
    );
    let mut global = format!(
        r#"<?xml version='1.0'?>
<!DOCTYPE fontconfig SYSTEM 'fonts.dtd'>
<fontconfig>
{hints}
    <!-- Keep the requested weight/slant. Remove source-specific style names,
         which otherwise take precedence over weight in Fontconfig matching. -->
    <match target="pattern">
        <edit name="style" mode="delete_all"/>
        <edit name="family" mode="assign_replace" binding="strong">
"#
    );
    if fonts.bold.is_some() {
        global.push_str(&format!(
            r#"            <if>
                <more_eq><name>weight</name><const>demibold</const></more_eq>
                <string>{safe_bold_name}</string>
                <string>{safe_font_name}</string>
            </if>
"#
        ));
    } else {
        global.push_str(&format!("            <string>{safe_font_name}</string>\n"));
    }
    global.push_str("        </edit>\n    </match>\n");
    // Match the deployed files, rather than family strings, and scale exactly once.
    for (face, bold) in [(Some(regular), false), (fonts.bold.as_ref(), true)] {
        if let Some(face) = face {
            let filename = face.deployed_name(bold);
            global.push_str(&format!(
                r#"    <match target="font">
        <test name="file" compare="contains"><string>{filename}</string></test>
        <edit name="pixelsize" mode="assign">
            <times><name>pixelsize</name><double>{ui_scale}</double></times>
        </edit>
    </match>
"#
            ));
        }
    }
    global.push_str("</fontconfig>");
    FontConfigs { local, global }
}

/// Named variants can arrive without a weight field. Never lower an explicit heavier request.
fn weight_hints() -> String {
    // Supply a default, then retain only the first requested weight. Leaving the
    // appended regular weight in the list makes Fontconfig 2.11 choose regular
    // even for a bold request when both files belong to the same family.
    let mut rules = String::from(
        r#"    <match target="pattern">
        <edit name="weight" mode="append"><const>regular</const></edit>
        <edit name="weight" mode="assign"><name>weight</name></edit>
    </match>
"#,
    );
    for (suffix, weight) in [
        (" SemiBold", "demibold"),
        (" Semibold", "demibold"),
        (" DemiBold", "demibold"),
        (" Bold", "bold"),
        (" ExtraBold", "extrabold"),
        (" Black", "black"),
        (" Heavy", "heavy"),
    ] {
        let mut tests = vec![("family", "contains", suffix.to_string())];
        for ending in ["", " Italic", " Oblique"] {
            tests.push(("style", "eq", format!("{}{ending}", suffix.trim())));
        }
        for (property, compare, value) in tests {
            rules.push_str(&format!(
                r#"    <match target="pattern">
        <test name="{property}" compare="{compare}"><string>{value}</string></test>
        <edit name="weight" mode="assign">
            <if>
                <more><name>weight</name><const>{weight}</const></more>
                <name>weight</name><const>{weight}</const>
            </if>
        </edit>
    </match>
"#
            ));
        }
    }
    for (word, slant) in [("Italic", "italic"), ("Oblique", "oblique")] {
        for property in ["family", "style"] {
            rules.push_str(&format!(
                r#"    <match target="pattern">
        <test name="{property}" compare="contains"><string>{word}</string></test>
        <edit name="slant" mode="assign"><const>{slant}</const></edit>
    </match>
"#
            ));
        }
    }
    rules
}
